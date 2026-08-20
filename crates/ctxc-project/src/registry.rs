//! The project registry.
//!
//! Registering a project is the one place several concerns meet: canonicalizing
//! the path so the same directory is never two projects, reading whatever the
//! project says about itself, detecting what it is made of, and recording it.
//!
//! Storage sits behind [`ProjectStore`] so that this logic can be tested
//! without a database, and so the daemon, the CLI and the HTTP API all go
//! through the same door.

use std::path::Path;

use ctxc_core::Timestamp;

use crate::config::ProjectFile;
use crate::detect;
use crate::error::{ProjectError, Result};
use crate::model::{Detection, Project, ProjectId, ProjectStatus};

/// Persistence for the registry.
pub trait ProjectStore {
    fn insert_project(&self, project: &Project) -> Result<()>;
    fn project(&self, id: &ProjectId) -> Result<Option<Project>>;
    fn project_by_path(&self, path: &str) -> Result<Option<Project>>;
    fn projects(&self) -> Result<Vec<Project>>;
    fn set_status(&self, id: &ProjectId, status: ProjectStatus) -> Result<bool>;
    fn set_detection(&self, id: &ProjectId, detection: &Detection) -> Result<bool>;
    fn record_indexed(&self, id: &ProjectId, at: Timestamp) -> Result<bool>;
    fn remove_project(&self, id: &ProjectId) -> Result<bool>;
    fn set_setting(&self, id: &ProjectId, key: &str, value: &str) -> Result<()>;
    fn setting(&self, id: &ProjectId, key: &str) -> Result<Option<String>>;
}

/// What registering a project produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    pub project: Project,
    /// False when the project was already registered.
    pub added: bool,
}

/// Registry operations, over any [`ProjectStore`].
pub struct Registry<'a> {
    store: &'a dyn ProjectStore,
}

impl<'a> Registry<'a> {
    pub fn new(store: &'a dyn ProjectStore) -> Self {
        Registry { store }
    }

    /// Register a directory as a project.
    ///
    /// Adding the same directory twice is not an error: it returns what is
    /// already registered, so scripts and agents can call it freely.
    pub fn add(&self, path: &Path) -> Result<Registration> {
        let root = canonical_root(path)?;
        if let Some(project) = self.store.project_by_path(&root)? {
            return Ok(Registration {
                project,
                added: false,
            });
        }

        let file = ProjectFile::load(Path::new(&root))?.map(|(_, file)| file);
        let added_at = Timestamp::now();

        let project = Project {
            // A project that states its own id keeps it across moves.
            id: file
                .as_ref()
                .and_then(ProjectFile::declared_id)
                .unwrap_or_else(|| ProjectId::generate(&root, added_at)),
            name: file
                .as_ref()
                .and_then(|file| file.project.name.clone())
                .unwrap_or_else(|| default_name(&root)),
            detection: detect::detect(Path::new(&root)),
            status: ProjectStatus::Active,
            added_at,
            last_indexed_at: None,
            path: root,
        };

        self.store.insert_project(&project)?;
        Ok(Registration {
            project,
            added: true,
        })
    }

    /// Every registered project, in the order they were added.
    pub fn list(&self) -> Result<Vec<Project>> {
        self.store.projects()
    }

    /// Projects the daemon should be looking after.
    pub fn active(&self) -> Result<Vec<Project>> {
        Ok(self
            .store
            .projects()?
            .into_iter()
            .filter(|project| project.status.is_active())
            .collect())
    }

    /// Find a project by id, by path, or by name.
    ///
    /// People refer to projects by whichever of these is in front of them, and
    /// making them find out which one the command wanted would be pointless.
    pub fn resolve(&self, reference: &str) -> Result<Project> {
        if let Some(id) = ProjectId::parse(reference) {
            if let Some(project) = self.store.project(&id)? {
                return Ok(project);
            }
        }

        if let Ok(root) = canonical_root(Path::new(reference)) {
            if let Some(project) = self.store.project_by_path(&root)? {
                return Ok(project);
            }
        }

        let by_name: Vec<Project> = self
            .store
            .projects()?
            .into_iter()
            .filter(|project| project.name == reference)
            .collect();

        match by_name.len() {
            1 => Ok(by_name.into_iter().next().expect("checked length")),
            0 => Err(ProjectError::NotRegistered {
                reference: reference.to_string(),
            }),
            _ => Err(ProjectError::Ambiguous {
                reference: reference.to_string(),
                matches: by_name.into_iter().map(|project| project.path).collect(),
            }),
        }
    }

    /// Stop looking after a project without forgetting it.
    pub fn pause(&self, reference: &str) -> Result<Project> {
        self.set_status(reference, ProjectStatus::Paused)
    }

    /// Start looking after it again.
    pub fn resume(&self, reference: &str) -> Result<Project> {
        self.set_status(reference, ProjectStatus::Active)
    }

    fn set_status(&self, reference: &str, status: ProjectStatus) -> Result<Project> {
        let mut project = self.resolve(reference)?;
        self.store.set_status(&project.id, status)?;
        project.status = status;
        Ok(project)
    }

    /// Forget a project.
    ///
    /// Only CtxC's own records go: the project's files are never touched, which
    /// is why this returns the project rather than asking for confirmation.
    pub fn remove(&self, reference: &str) -> Result<Project> {
        let project = self.resolve(reference)?;
        self.store.remove_project(&project.id)?;
        Ok(project)
    }

    /// Re-run detection, for a project that has changed shape.
    pub fn refresh_detection(&self, reference: &str) -> Result<Project> {
        let mut project = self.resolve(reference)?;
        let detection = detect::detect(Path::new(&project.path));
        self.store.set_detection(&project.id, &detection)?;
        project.detection = detection;
        Ok(project)
    }

    /// Note that a project has just been indexed.
    pub fn record_indexed(&self, id: &ProjectId, at: Timestamp) -> Result<()> {
        self.store.record_indexed(id, at)?;
        Ok(())
    }
}

/// The canonical form of a project root: absolute, forward slashes, no verbatim
/// prefix. The index uses the same form, so the two always agree.
pub fn canonical_root(path: &Path) -> Result<String> {
    let canonical = std::fs::canonicalize(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            ProjectError::NoSuchDirectory {
                path: path.to_path_buf(),
            }
        } else {
            ProjectError::Io {
                action: "read directory",
                path: path.to_path_buf(),
                source,
            }
        }
    })?;

    if !canonical.is_dir() {
        return Err(ProjectError::NotADirectory {
            path: path.to_path_buf(),
        });
    }

    let text = canonical.to_string_lossy();
    Ok(text
        .strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .replace('\\', "/"))
}

/// The last path segment, which is what people call the project.
fn default_name(root: &str) -> String {
    root.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(root)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// An in-memory store, so registry behaviour is tested without SQLite.
    #[derive(Default)]
    struct MemoryStore {
        projects: RefCell<Vec<Project>>,
        settings: RefCell<HashMap<(String, String), String>>,
    }

    impl ProjectStore for MemoryStore {
        fn insert_project(&self, project: &Project) -> Result<()> {
            self.projects.borrow_mut().push(project.clone());
            Ok(())
        }

        fn project(&self, id: &ProjectId) -> Result<Option<Project>> {
            Ok(self
                .projects
                .borrow()
                .iter()
                .find(|project| project.id == *id)
                .cloned())
        }

        fn project_by_path(&self, path: &str) -> Result<Option<Project>> {
            Ok(self
                .projects
                .borrow()
                .iter()
                .find(|project| project.path == path)
                .cloned())
        }

        fn projects(&self) -> Result<Vec<Project>> {
            Ok(self.projects.borrow().clone())
        }

        fn set_status(&self, id: &ProjectId, status: ProjectStatus) -> Result<bool> {
            let mut projects = self.projects.borrow_mut();
            match projects.iter_mut().find(|project| project.id == *id) {
                Some(project) => {
                    project.status = status;
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        fn set_detection(&self, id: &ProjectId, detection: &Detection) -> Result<bool> {
            let mut projects = self.projects.borrow_mut();
            match projects.iter_mut().find(|project| project.id == *id) {
                Some(project) => {
                    project.detection = detection.clone();
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        fn record_indexed(&self, id: &ProjectId, at: Timestamp) -> Result<bool> {
            let mut projects = self.projects.borrow_mut();
            match projects.iter_mut().find(|project| project.id == *id) {
                Some(project) => {
                    project.last_indexed_at = Some(at);
                    Ok(true)
                }
                None => Ok(false),
            }
        }

        fn remove_project(&self, id: &ProjectId) -> Result<bool> {
            let mut projects = self.projects.borrow_mut();
            let before = projects.len();
            projects.retain(|project| project.id != *id);
            Ok(projects.len() < before)
        }

        fn set_setting(&self, id: &ProjectId, key: &str, value: &str) -> Result<()> {
            self.settings
                .borrow_mut()
                .insert((id.to_string(), key.to_string()), value.to_string());
            Ok(())
        }

        fn setting(&self, id: &ProjectId, key: &str) -> Result<Option<String>> {
            Ok(self
                .settings
                .borrow()
                .get(&(id.to_string(), key.to_string()))
                .cloned())
        }
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let path = std::env::temp_dir()
                .join("ctxc-registry-tests")
                .join(format!("{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Fixture(path)
        }

        fn project(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn adding_a_project_records_and_detects_it() {
        let fixture = Fixture::new("add");
        let path = fixture.project("acme-web");
        std::fs::write(path.join("Cargo.toml"), "").unwrap();

        let store = MemoryStore::default();
        let registration = Registry::new(&store).add(&path).unwrap();

        assert!(registration.added);
        assert_eq!(registration.project.name, "acme-web");
        assert_eq!(registration.project.status, ProjectStatus::Active);
        assert_eq!(registration.project.detection.languages, vec!["rust"]);
        assert!(!registration.project.path.contains('\\'));
    }

    #[test]
    fn adding_the_same_project_twice_is_harmless() {
        let fixture = Fixture::new("twice");
        let path = fixture.project("demo");
        let store = MemoryStore::default();
        let registry = Registry::new(&store);

        let first = registry.add(&path).unwrap();
        let second = registry.add(&path).unwrap();

        assert!(first.added);
        assert!(!second.added);
        assert_eq!(first.project.id, second.project.id);
        assert_eq!(registry.list().unwrap().len(), 1);
    }

    #[test]
    fn a_project_can_declare_its_own_identity_and_name() {
        let fixture = Fixture::new("declared");
        let path = fixture.project("whatever");
        std::fs::write(
            path.join(".ctxc.toml"),
            "[project]\nid = \"acme-web\"\nname = \"Acme Web\"\n",
        )
        .unwrap();

        let store = MemoryStore::default();
        let project = Registry::new(&store).add(&path).unwrap().project;

        assert_eq!(project.id.as_str(), "acme-web");
        assert_eq!(project.name, "Acme Web");
    }

    #[test]
    fn registering_never_writes_to_the_project() {
        let fixture = Fixture::new("read-only");
        let path = fixture.project("untouched");
        let store = MemoryStore::default();

        Registry::new(&store).add(&path).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&path).unwrap().collect();
        assert!(entries.is_empty(), "adding a project must not create files");
    }

    #[test]
    fn projects_resolve_by_id_path_or_name() {
        let fixture = Fixture::new("resolve");
        let path = fixture.project("acme");
        let store = MemoryStore::default();
        let registry = Registry::new(&store);
        let project = registry.add(&path).unwrap().project;

        assert_eq!(
            registry.resolve(project.id.as_str()).unwrap().id,
            project.id
        );
        assert_eq!(
            registry.resolve(path.to_str().unwrap()).unwrap().id,
            project.id
        );
        assert_eq!(registry.resolve("acme").unwrap().id, project.id);
    }

    #[test]
    fn an_unknown_reference_says_so() {
        let store = MemoryStore::default();
        let error = Registry::new(&store)
            .resolve("nothing-like-this")
            .unwrap_err();

        assert!(matches!(error, ProjectError::NotRegistered { .. }));
        assert!(error.hint().unwrap().contains("ctxc project list"));
    }

    #[test]
    fn pausing_takes_a_project_out_of_the_active_set() {
        let fixture = Fixture::new("pause");
        let path = fixture.project("paused-one");
        let store = MemoryStore::default();
        let registry = Registry::new(&store);
        registry.add(&path).unwrap();

        assert_eq!(registry.active().unwrap().len(), 1);
        let paused = registry.pause("paused-one").unwrap();
        assert_eq!(paused.status, ProjectStatus::Paused);
        assert!(registry.active().unwrap().is_empty());

        registry.resume("paused-one").unwrap();
        assert_eq!(registry.active().unwrap().len(), 1);
    }

    #[test]
    fn removing_forgets_the_project_but_not_its_files() {
        let fixture = Fixture::new("remove");
        let path = fixture.project("goodbye");
        std::fs::write(path.join("keep.txt"), "still here").unwrap();

        let store = MemoryStore::default();
        let registry = Registry::new(&store);
        registry.add(&path).unwrap();
        registry.remove("goodbye").unwrap();

        assert!(registry.list().unwrap().is_empty());
        assert!(path.join("keep.txt").exists(), "files must survive");
    }

    #[test]
    fn detection_can_be_re_run_after_a_project_changes() {
        let fixture = Fixture::new("refresh");
        let path = fixture.project("growing");
        let store = MemoryStore::default();
        let registry = Registry::new(&store);

        let before = registry.add(&path).unwrap().project;
        assert!(before.detection.is_empty());

        std::fs::write(path.join("go.mod"), "module demo").unwrap();
        let after = registry.refresh_detection("growing").unwrap();
        assert_eq!(after.detection.languages, vec!["go"]);
    }

    #[test]
    fn a_missing_directory_cannot_be_registered() {
        let store = MemoryStore::default();
        let error = Registry::new(&store)
            .add(Path::new("no-such-project-directory"))
            .unwrap_err();

        assert!(matches!(error, ProjectError::NoSuchDirectory { .. }));
    }

    #[test]
    fn a_file_cannot_be_registered() {
        let fixture = Fixture::new("file");
        let path = fixture.0.join("a-file.txt");
        std::fs::write(&path, "").unwrap();

        let store = MemoryStore::default();
        let error = Registry::new(&store).add(&path).unwrap_err();
        assert!(matches!(error, ProjectError::NotADirectory { .. }));
    }

    #[test]
    fn the_same_directory_reached_differently_is_one_project() {
        let fixture = Fixture::new("canonical");
        let path = fixture.project("single");
        let store = MemoryStore::default();
        let registry = Registry::new(&store);

        registry.add(&path).unwrap();
        let indirect = registry.add(&path.join("..").join("single")).unwrap();

        assert!(!indirect.added);
        assert_eq!(registry.list().unwrap().len(), 1);
    }
}
