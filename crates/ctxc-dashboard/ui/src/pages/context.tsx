/**
 * Searching the index, and reading what came back.
 *
 * This is `ctxc find`, both halves of it — the search and the reference
 * recovery — with a screen instead of a terminal: the same routes, the same
 * ranking, the same stored originals. What
 * a screen adds is being able to open a result and read it without leaving the
 * list, and seeing *why* each file was chosen rather than only that it was.
 *
 * Everything rendered here is repository content — file paths, code, error
 * text. React escapes all of it, and nothing on this page is ever inserted as
 * markup.
 */

import * as React from "react";
import {
  ArrowRight,
  FileCode,
  FileSearch,
  Link2,
  Network,
  Search as SearchIcon,
} from "lucide-react";

import {
  api,
  type Reason,
  type Retrieval,
  type RetrievedFile,
  type StoredContext,
} from "../lib/api";
import { bytes, compact, exact } from "../lib/format";
import { href, Link } from "../lib/router";
import { cn } from "../lib/utils";
import { hintOf, useApi, useMutation } from "../lib/useApi";
import { PageBody, PageHeader, Section } from "../components/page";
import { useScope } from "../components/scope";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardHeading,
  CardTitle,
  Code,
  Empty,
  Failure,
  Input,
  Label,
  ScrollArea,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "../components/ui";

export function Context({ revision }: { revision: number }) {
  return (
    <PageBody>
      <PageHeader
        title="Context"
        description="Find the files a question actually needs, and recover the original behind any optimized output."
      />

      <Tabs defaultValue="search">
        <TabsList className="mb-4">
          <TabsTrigger value="search">
            <SearchIcon /> Search
          </TabsTrigger>
          <TabsTrigger value="reference">
            <Link2 /> Reference
          </TabsTrigger>
          <TabsTrigger value="graph">
            <Network /> Dependencies
          </TabsTrigger>
        </TabsList>

        <TabsContent value="search">
          <SearchPanel />
        </TabsContent>
        <TabsContent value="reference">
          <ReferencePanel />
        </TabsContent>
        <TabsContent value="graph">
          <GraphPanel revision={revision} />
        </TabsContent>
      </Tabs>
    </PageBody>
  );
}

// ----------------------------------------------------------------- search

function SearchPanel() {
  const { project, setProject, projects } = useScope();
  const [query, setQuery] = React.useState("");
  const [limit, setLimit] = React.useState(20);
  const [opened, setOpened] = React.useState<string | null>(null);

  // The result is held here rather than in a query hook on purpose: a search is
  // something a person asked for once. Re-running it on every stream event
  // would rewrite the list while they were reading it.
  const [retrieval, setRetrieval] = React.useState<Retrieval | null>(null);

  const search = useMutation(
    (target: string, text: string, count: number) =>
      api.search(target, text, count),
    {
      onDone: (result) => {
        setRetrieval(result);
        setOpened(null);
      },
    },
  );

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const text = query.trim();
    if (!project || !text) return;
    void search.run(project, text, limit);
  };

  if (projects.length === 0) {
    return (
      <Card>
        <Empty
          icon={FileSearch}
          title="There is nothing to search yet."
          hint="Register a project and let it be indexed; search reads the index, not the disk."
          action={
            <Button size="sm" asChild>
              <Link to={href("/projects")}>Add a project</Link>
            </Button>
          }
        />
      </Card>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardContent className="px-5 py-4">
          <form onSubmit={submit} className="flex flex-wrap items-end gap-3">
            <div className="min-w-64 flex-1 space-y-2">
              <Label htmlFor="search-query">What are you looking for?</Label>
              <Input
                id="search-query"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="auth timeout, or a quoted phrase"
                spellCheck={false}
                autoFocus
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="search-project">Project</Label>
              <Select
                value={project ?? ""}
                onValueChange={(value) => setProject(value)}
              >
                <SelectTrigger id="search-project" className="w-48">
                  <SelectValue placeholder="Choose one" />
                </SelectTrigger>
                <SelectContent>
                  {projects.map((candidate) => (
                    <SelectItem key={candidate.id} value={candidate.id}>
                      {candidate.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label htmlFor="search-limit">Results</Label>
              <Select
                value={String(limit)}
                onValueChange={(value) => setLimit(Number(value))}
              >
                <SelectTrigger id="search-limit" className="w-24">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {[10, 20, 50].map((count) => (
                    <SelectItem key={count} value={String(count)}>
                      {count}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <Button
              type="submit"
              disabled={search.pending || !project || !query.trim()}
            >
              <SearchIcon />
              {search.pending ? "Searching…" : "Search"}
            </Button>
          </form>

          <p className="text-muted-foreground mt-3 text-xs">
            Quoting a phrase keeps it together, the same way{" "}
            <Code>ctxc find</Code> treats it.
          </p>
        </CardContent>
      </Card>

      {search.error ? (
        <Failure message={search.error.message} hint={hintOf(search.error)} />
      ) : null}

      {search.pending ? (
        <Card>
          <CardContent className="space-y-2 px-5 py-4">
            {Array.from({ length: 5 }, (_, index) => (
              <Skeleton key={index} className="h-16" />
            ))}
          </CardContent>
        </Card>
      ) : retrieval ? (
        <SearchResults
          retrieval={retrieval}
          project={project}
          opened={opened}
          onOpen={setOpened}
        />
      ) : null}
    </div>
  );
}

function SearchResults({
  retrieval,
  project,
  opened,
  onOpen,
}: {
  retrieval: Retrieval;
  project?: string;
  opened: string | null;
  onOpen: (path: string | null) => void;
}) {
  if (retrieval.files.length === 0) {
    return (
      <Card>
        <Empty
          icon={FileSearch}
          title={`Nothing matched ${retrieval.query}.`}
          hint={`${exact(retrieval.considered)} files were considered. Try fewer words, or a symbol name.`}
        />
      </Card>
    );
  }

  return (
    <div className="grid gap-6 xl:grid-cols-5">
      <Card className="xl:col-span-2">
        <CardHeader>
          <CardHeading>
            <CardTitle>{exact(retrieval.files.length)} files</CardTitle>
            <CardDescription>
              Ranked out of {exact(retrieval.considered)} considered.
            </CardDescription>
          </CardHeading>
        </CardHeader>
        <CardContent className="px-2 pb-2">
          <ScrollArea className="max-h-[36rem]">
            <ul className="space-y-1 pr-2">
              {retrieval.files.map((file) => (
                <li key={file.path}>
                  <button
                    onClick={() =>
                      onOpen(opened === file.path ? null : file.path)
                    }
                    className={cn(
                      "w-full rounded-md px-3 py-2.5 text-left transition-colors",
                      opened === file.path
                        ? "bg-primary/10 ring-primary/30 ring-1"
                        : "hover:bg-muted/60",
                    )}
                  >
                    <ResultSummary file={file} />
                  </button>
                </li>
              ))}
            </ul>
          </ScrollArea>
        </CardContent>
      </Card>

      <div className="xl:col-span-3">
        {opened && project ? (
          <FileViewer project={project} path={opened} />
        ) : (
          <Card className="h-full">
            <Empty
              icon={FileCode}
              title="Choose a file to read it."
              hint="Content comes from the index, so it is what CtxC would actually give an agent."
            />
          </Card>
        )}
      </div>
    </div>
  );
}

function ResultSummary({ file }: { file: RetrievedFile }) {
  const firstLine = file.snippet?.split("\n")[0];

  return (
    <>
      <span className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate font-mono text-xs">
          {file.path}
        </span>
        <span className="tabular text-muted-foreground shrink-0 text-xs">
          {file.score.toFixed(2)}
        </span>
      </span>
      <span className="mt-1.5 flex flex-wrap items-center gap-1">
        <ReasonBadge reason={file.reason} />
        {file.language ? <Badge variant="outline">{file.language}</Badge> : null}
        {file.matched_symbols.slice(0, 2).map((symbol) => (
          <Badge key={symbol} variant="primary" className="font-mono">
            {symbol}
          </Badge>
        ))}
      </span>
      {firstLine ? (
        <span className="text-muted-foreground mt-1.5 block truncate font-mono text-[0.7rem]">
          {file.line ? `${file.line}: ` : ""}
          {firstLine}
        </span>
      ) : null}
    </>
  );
}

function ReasonBadge({ reason }: { reason: Reason }) {
  switch (reason.kind) {
    case "content":
      return <Badge variant="info">Content match</Badge>;
    case "symbol":
      return <Badge variant="success">Symbol match</Badge>;
    case "content_and_symbol":
      return <Badge variant="success">Content and symbol</Badge>;
    case "related":
      return (
        <Badge variant="outline">
          {reason.hops} hop{reason.hops === 1 ? "" : "s"} from {reason.to}
        </Badge>
      );
  }
}

function FileViewer({ project, path }: { project: string; path: string }) {
  const file = useApi(() => api.projectFile(project, path), [project, path]);

  return (
    <Card className="h-full">
      <CardHeader>
        <CardHeading>
          <CardTitle className="font-mono text-xs break-all">{path}</CardTitle>
          <CardDescription>
            {file.data
              ? [
                  `${exact(file.data.content.length)} characters`,
                  file.data.language,
                  `${file.data.dependencies.length} dependencies`,
                ]
                  .filter(Boolean)
                  .join(" · ")
              : "Reading from the index…"}
          </CardDescription>
        </CardHeading>
      </CardHeader>
      <CardContent>
        {file.error ? (
          <Failure
            message={file.error.message}
            hint={hintOf(file.error)}
            onRetry={file.reload}
          />
        ) : file.loading ? (
          <Skeleton className="h-80" />
        ) : file.data ? (
          <div className="space-y-4">
            {file.data.dependencies.length > 0 ||
            file.data.dependents.length > 0 ? (
              <div className="grid gap-3 sm:grid-cols-2">
                <Relations title="Depends on" paths={file.data.dependencies} />
                <Relations title="Depended on by" paths={file.data.dependents} />
              </div>
            ) : null}

            <pre className="bg-muted/40 max-h-[30rem] overflow-auto rounded-md border p-3 font-mono text-xs leading-relaxed">
              {file.data.content}
            </pre>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

function Relations({ title, paths }: { title: string; paths: string[] }) {
  return (
    <div className="space-y-1.5">
      <p className="text-muted-foreground text-xs font-medium">
        {title} ({paths.length})
      </p>
      {paths.length === 0 ? (
        <p className="text-muted-foreground text-xs">Nothing resolved.</p>
      ) : (
        <ul className="space-y-0.5">
          {paths.slice(0, 6).map((entry) => (
            <li key={entry} className="truncate font-mono text-[0.7rem]">
              {entry}
            </li>
          ))}
          {paths.length > 6 ? (
            <li className="text-muted-foreground text-[0.7rem]">
              and {paths.length - 6} more
            </li>
          ) : null}
        </ul>
      )}
    </div>
  );
}

// -------------------------------------------------------------- reference

function ReferencePanel() {
  const [reference, setReference] = React.useState("");
  const [found, setFound] = React.useState<StoredContext | null>(null);

  const open = useMutation((value: string) => api.context(value), {
    onDone: setFound,
  });

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const value = reference.trim();
    if (value) void open.run(value);
  };

  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardContent className="px-5 py-4">
          <form onSubmit={submit} className="flex flex-wrap items-end gap-3">
            <div className="min-w-64 flex-1 space-y-2">
              <Label htmlFor="reference">Context reference</Label>
              <Input
                id="reference"
                value={reference}
                onChange={(event) => setReference(event.target.value)}
                placeholder="ctxc://context/9f2c1d…"
                spellCheck={false}
                className="font-mono"
              />
            </div>
            <Button type="submit" disabled={open.pending || !reference.trim()}>
              <ArrowRight />
              {open.pending ? "Opening…" : "Open"}
            </Button>
          </form>
          <p className="text-muted-foreground mt-3 text-xs">
            Optimized output carries one of these. Opening it gives back the
            original, exactly as <Code>ctxc find</Code> would. The id on its
            own works too.
          </p>
        </CardContent>
      </Card>

      {open.error ? (
        <Failure message={open.error.message} hint={hintOf(open.error)} />
      ) : null}

      {found ? (
        <Card>
          <CardHeader>
            <CardHeading>
              <CardTitle className="font-mono text-xs break-all">
                {found.reference}
              </CardTitle>
              <CardDescription>
                {found.source} · {found.content_type} · {bytes(found.bytes)}
              </CardDescription>
            </CardHeading>
          </CardHeader>
          <CardContent>
            <pre className="bg-muted/40 max-h-[32rem] overflow-auto rounded-md border p-3 font-mono text-xs leading-relaxed">
              {found.content}
            </pre>
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}

// ------------------------------------------------------------------ graph

function GraphPanel({ revision }: { revision: number }) {
  const { project, projects, selected } = useScope();
  const graph = useApi(
    () => (project ? api.projectGraph(project, 20) : Promise.resolve(null)),
    [project, revision],
  );

  if (projects.length === 0) {
    return (
      <Card>
        <Empty
          icon={Network}
          title="No projects to graph."
          hint="Register one, and CtxC records the relationships it can resolve while indexing."
        />
      </Card>
    );
  }

  if (!project) {
    return (
      <Card>
        <Empty
          icon={Network}
          title="Choose a project."
          hint="Use the project picker at the top of the page. A dependency graph belongs to one project."
        />
      </Card>
    );
  }

  return (
    <Section
      title={`${selected?.name ?? "Project"} dependencies`}
      description="The same summary `ctxc project graph` prints."
    >
      <Card>
        <CardContent className="px-5 py-4">
          {graph.error ? (
            <Failure
              message={graph.error.message}
              hint={hintOf(graph.error)}
              onRetry={graph.reload}
            />
          ) : graph.loading ? (
            <Skeleton className="h-64" />
          ) : graph.data && graph.data.most_depended_on.length > 0 ? (
            <>
              <div className="mb-4 flex flex-wrap gap-6 text-sm">
                <span>
                  <span className="tabular font-semibold">
                    {exact(graph.data.files)}
                  </span>{" "}
                  <span className="text-muted-foreground">files in the graph</span>
                </span>
                <span>
                  <span className="tabular font-semibold">
                    {exact(graph.data.edges)}
                  </span>{" "}
                  <span className="text-muted-foreground">
                    resolved relationships
                  </span>
                </span>
              </div>
              <ul className="divide-border/60 divide-y">
                {graph.data.most_depended_on.map((entry) => (
                  <li
                    key={entry.path}
                    className="flex items-center gap-3 py-2 text-sm"
                  >
                    <span className="min-w-0 flex-1 truncate font-mono text-xs">
                      {entry.path}
                    </span>
                    <Badge variant="outline">
                      {compact(entry.dependents)} dependents
                    </Badge>
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <Empty
              icon={Network}
              title="No relationships resolved yet."
              hint="Index the project. CtxC records imports it can resolve to files it has already seen."
            />
          )}
        </CardContent>
      </Card>
    </Section>
  );
}
