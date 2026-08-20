//! SQLite implementation of the project registry.
//!
//! The interface it satisfies lives with the registry logic in `ctxc-project`;
//! this is only the adapter, which is what keeps the rules about identity and
//! status out of SQL.

use rusqlite::{Connection, OptionalExtension};

use ctxc_core::Timestamp;
use ctxc_project::model::{Detection, Project, ProjectId, ProjectStatus};
use ctxc_project::registry::ProjectStore;
use ctxc_project::{ProjectError, Result as ProjectResult};

use crate::db::Database;

/// SQLite-backed [`ProjectStore`].
pub struct SqliteProjectStore<'a> {
    conn: &'a Connection,
}

impl<'a> SqliteProjectStore<'a> {
    pub fn new(database: &'a Database) -> Self {
        SqliteProjectStore {
            conn: database.connection(),
        }
    }
}

/// Turn a database failure into something the registry can carry.
///
/// The registry's error type deliberately knows nothing about SQLite; the
/// message survives, the type does not leak.
fn storage(error: rusqlite::Error) -> ProjectError {
    ProjectError::Storage(format!("project registry: {error}"))
}

/// Lists are stored as comma-separated text: they are short, read whole, and
/// never queried by element.
fn join(values: &[String]) -> String {
    values.join(",")
}

fn split(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(String::from)
        .collect()
}

impl ProjectStore for SqliteProjectStore<'_> {
    fn insert_project(&self, project: &Project) -> ProjectResult<()> {
        self.conn
            .execute(
                "INSERT INTO projects
                     (id, name, path, status, added_at, last_indexed_at,
                      languages, frameworks, package_manager, has_git)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    project.id.as_str(),
                    project.name,
                    project.path,
                    project.status.as_str(),
                    project.added_at.as_millis(),
                    project.last_indexed_at.map(Timestamp::as_millis),
                    join(&project.detection.languages),
                    join(&project.detection.frameworks),
                    project.detection.package_manager,
                    project.detection.git as i64,
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn project(&self, id: &ProjectId) -> ProjectResult<Option<Project>> {
        self.query_one("WHERE id = ?1", [id.as_str()])
    }

    fn project_by_path(&self, path: &str) -> ProjectResult<Option<Project>> {
        self.query_one("WHERE path = ?1", [path])
    }

    fn projects(&self) -> ProjectResult<Vec<Project>> {
        let mut statement = self
            .conn
            .prepare(&format!("{SELECT} ORDER BY added_at, name"))
            .map_err(storage)?;
        let projects = statement
            .query_map([], read_project)
            .map_err(storage)?
            .collect::<rusqlite::Result<Vec<Project>>>()
            .map_err(storage)?;
        Ok(projects)
    }

    fn set_status(&self, id: &ProjectId, status: ProjectStatus) -> ProjectResult<bool> {
        let updated = self
            .conn
            .execute(
                "UPDATE projects SET status = ?2 WHERE id = ?1",
                rusqlite::params![id.as_str(), status.as_str()],
            )
            .map_err(storage)?;
        Ok(updated > 0)
    }

    fn set_detection(&self, id: &ProjectId, detection: &Detection) -> ProjectResult<bool> {
        let updated = self
            .conn
            .execute(
                "UPDATE projects
                 SET languages = ?2, frameworks = ?3, package_manager = ?4, has_git = ?5
                 WHERE id = ?1",
                rusqlite::params![
                    id.as_str(),
                    join(&detection.languages),
                    join(&detection.frameworks),
                    detection.package_manager,
                    detection.git as i64,
                ],
            )
            .map_err(storage)?;
        Ok(updated > 0)
    }

    fn record_indexed(&self, id: &ProjectId, at: Timestamp) -> ProjectResult<bool> {
        let updated = self
            .conn
            .execute(
                "UPDATE projects SET last_indexed_at = ?2 WHERE id = ?1",
                rusqlite::params![id.as_str(), at.as_millis()],
            )
            .map_err(storage)?;
        Ok(updated > 0)
    }

    fn remove_project(&self, id: &ProjectId) -> ProjectResult<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM projects WHERE id = ?1", [id.as_str()])
            .map_err(storage)?;
        Ok(removed > 0)
    }

    fn set_setting(&self, id: &ProjectId, key: &str, value: &str) -> ProjectResult<()> {
        self.conn
            .execute(
                "INSERT INTO project_settings (project_id, key, value)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(project_id, key) DO UPDATE SET value = excluded.value",
                rusqlite::params![id.as_str(), key, value],
            )
            .map_err(storage)?;
        Ok(())
    }

    fn setting(&self, id: &ProjectId, key: &str) -> ProjectResult<Option<String>> {
        let value = self
            .conn
            .query_row(
                "SELECT value FROM project_settings WHERE project_id = ?1 AND key = ?2",
                [id.as_str(), key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage)?;
        Ok(value)
    }
}

const SELECT: &str = "SELECT id, name, path, status, added_at, last_indexed_at,
                             languages, frameworks, package_manager, has_git
                      FROM projects";

impl SqliteProjectStore<'_> {
    fn query_one<P: rusqlite::Params>(
        &self,
        filter: &str,
        params: P,
    ) -> ProjectResult<Option<Project>> {
        let project = self
            .conn
            .query_row(&format!("{SELECT} {filter}"), params, read_project)
            .optional()
            .map_err(storage)?;
        Ok(project)
    }
}

fn read_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: ProjectId::parse(&row.get::<_, String>(0)?).unwrap_or_else(|| {
            // Ids are validated on the way in; a row that fails now is from a
            // future build, and showing it is better than hiding the project.
            ProjectId::parse("unreadable").expect("constant is a valid id")
        }),
        name: row.get(1)?,
        path: row.get(2)?,
        status: ProjectStatus::from_str_lossy(&row.get::<_, String>(3)?),
        added_at: Timestamp::from_millis(row.get(4)?),
        last_indexed_at: row.get::<_, Option<i64>>(5)?.map(Timestamp::from_millis),
        detection: Detection {
            languages: split(&row.get::<_, String>(6)?),
            frameworks: split(&row.get::<_, String>(7)?),
            package_manager: row.get(8)?,
            git: row.get::<_, i64>(9)? != 0,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, path: &str) -> Project {
        Project {
            id: ProjectId::parse(id).unwrap(),
            name: id.to_string(),
            path: path.to_string(),
            status: ProjectStatus::Active,
            added_at: Timestamp::from_millis(1_700_000_000_000),
            last_indexed_at: None,
            detection: Detection {
                languages: vec!["rust".into(), "typescript".into()],
                frameworks: vec!["next.js".into()],
                package_manager: Some("cargo".into()),
                git: true,
            },
        }
    }

    #[test]
    fn projects_round_trip() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteProjectStore::new(&database);
        let project = sample("acme-web", "/repo/acme-web");

        store.insert_project(&project).unwrap();
        let loaded = store.project(&project.id).unwrap().unwrap();

        assert_eq!(loaded, project);
        assert_eq!(
            store.project_by_path("/repo/acme-web").unwrap().unwrap(),
            project
        );
    }

    #[test]
    fn the_same_path_cannot_be_registered_twice() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteProjectStore::new(&database);

        store.insert_project(&sample("one", "/repo/same")).unwrap();
        let error = store
            .insert_project(&sample("two", "/repo/same"))
            .unwrap_err();

        assert!(matches!(error, ProjectError::Storage(_)));
    }

    #[test]
    fn status_detection_and_index_time_can_be_updated() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteProjectStore::new(&database);
        let project = sample("acme", "/repo/acme");
        store.insert_project(&project).unwrap();

        assert!(store
            .set_status(&project.id, ProjectStatus::Paused)
            .unwrap());
        assert!(store
            .set_detection(&project.id, &Detection::default())
            .unwrap());
        let at = Timestamp::from_millis(1_800_000_000_000);
        assert!(store.record_indexed(&project.id, at).unwrap());

        let loaded = store.project(&project.id).unwrap().unwrap();
        assert_eq!(loaded.status, ProjectStatus::Paused);
        assert!(loaded.detection.is_empty());
        assert_eq!(loaded.last_indexed_at, Some(at));
    }

    #[test]
    fn updates_to_an_unknown_project_report_that_nothing_changed() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteProjectStore::new(&database);
        let ghost = ProjectId::parse("ghost").unwrap();

        assert!(!store.set_status(&ghost, ProjectStatus::Paused).unwrap());
        assert!(!store.remove_project(&ghost).unwrap());
        assert!(store.project(&ghost).unwrap().is_none());
    }

    #[test]
    fn projects_come_back_in_the_order_they_were_added() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteProjectStore::new(&database);

        for (index, name) in ["first", "second", "third"].iter().enumerate() {
            let mut project = sample(name, &format!("/repo/{name}"));
            project.added_at = Timestamp::from_millis(1_700_000_000_000 + index as i64 * 1_000);
            store.insert_project(&project).unwrap();
        }

        let names: Vec<String> = store
            .projects()
            .unwrap()
            .into_iter()
            .map(|project| project.name)
            .collect();
        assert_eq!(names, vec!["first", "second", "third"]);
    }

    #[test]
    fn settings_are_per_project_and_overwrite() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteProjectStore::new(&database);
        let project = sample("acme", "/repo/acme");
        store.insert_project(&project).unwrap();

        store.set_setting(&project.id, "watch", "false").unwrap();
        store.set_setting(&project.id, "watch", "true").unwrap();

        assert_eq!(
            store.setting(&project.id, "watch").unwrap().as_deref(),
            Some("true")
        );
        assert!(store.setting(&project.id, "missing").unwrap().is_none());
    }

    #[test]
    fn removing_a_project_takes_its_settings_with_it() {
        let database = Database::open_in_memory().unwrap();
        let store = SqliteProjectStore::new(&database);
        let project = sample("acme", "/repo/acme");
        store.insert_project(&project).unwrap();
        store.set_setting(&project.id, "watch", "false").unwrap();

        assert!(store.remove_project(&project.id).unwrap());
        assert!(store.setting(&project.id, "watch").unwrap().is_none());
    }
}
