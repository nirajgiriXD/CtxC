/**
 * The configuration file, edited safely.
 *
 * There is one configuration in CtxC and it lives in a file. This screen does
 * not hold a second copy of it: it reads the layers the daemon resolved, writes
 * back only the keys someone actually changed, and lets the daemon validate the
 * result. A rejected edit leaves the file exactly as it was, and the message
 * shown is the daemon's own — the same words `ctxc config` would have printed.
 *
 * Three things are stated rather than hidden, because a settings screen that
 * lies about them is worse than no settings screen:
 *
 *  * a value set by a `CTXC_*` variable cannot be changed here, so its control
 *    is disabled rather than quietly ineffective;
 *  * a value written in the file is marked, and can be handed back to the
 *    default;
 *  * the daemon reads its configuration once, at startup, so an edit that it
 *    has not picked up yet says so.
 */

import * as React from "react";
import {
  Cog,
  Database,
  Gauge,
  Info,
  RotateCcw,
  Save,
  Scale,
  Server,
  Eye,
  ShieldCheck,
} from "lucide-react";

import { api, type Config, type ConfigView, type PartialConfig } from "../lib/api";
import { hintOf, useApi, useMutation } from "../lib/useApi";
import { PageBody, PageHeader } from "../components/page";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardHeading,
  CardTitle,
  Code,
  Failure,
  Field,
  FieldRow,
  Input,
  Notice,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  SkeletonRows,
  Switch,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  toast,
  Tooltip,
} from "../components/ui";

// ------------------------------------------------------------- the schema

type FieldSpec =
  | { kind: "switch"; key: string; label: string; description: string }
  | {
      kind: "number";
      key: string;
      label: string;
      description: string;
      unit?: string;
      step?: string;
    }
  | { kind: "text"; key: string; label: string; description: string }
  | {
      kind: "select";
      key: string;
      label: string;
      description: string;
      options: { value: string; label: string }[];
    };

interface Group {
  id: string;
  title: string;
  summary: string;
  icon: React.ComponentType<{ className?: string }>;
  fields: FieldSpec[];
  /** Shown above the controls when there is something worth warning about. */
  note?: React.ReactNode;
}

/**
 * Every setting the daemon actually supports, and nothing else.
 *
 * The daemon publishes the editable keys; a test in the dashboard would be
 * nice, but the honest check is that this list is derived from the same
 * `config.rs` the daemon validates against — anything invented here is refused
 * by the route, loudly, rather than silently ignored.
 */
const GROUPS: Group[] = [
  {
    id: "general",
    title: "General",
    summary: "What CtxC does by default, before anything asks it for more.",
    icon: Cog,
    fields: [
      {
        kind: "number",
        key: "budget.default",
        label: "Default token budget",
        description:
          "Applied when a command does not pass --budget. Optimized output is trimmed to fit it.",
        unit: "tokens",
      },
      {
        kind: "switch",
        key: "optimization.enabled",
        label: "Optimize content",
        description:
          "With this off, CtxC passes content through unchanged and still records what it saw.",
      },
      {
        kind: "number",
        key: "optimization.target_reduction",
        label: "Target reduction",
        description:
          "The fraction of the input the engine aims to remove. Between 0 and 1.",
        step: "0.05",
      },
      {
        kind: "switch",
        key: "retrieval.enabled",
        label: "Retrieval",
        description: "Whether search may select context from the index.",
      },
      {
        kind: "switch",
        key: "graph.enabled",
        label: "Dependency graph",
        description:
          "Record how files relate while indexing. Ranking uses it to reach files a query did not name.",
      },
      {
        kind: "switch",
        key: "telemetry.enabled",
        label: "External telemetry",
        description:
          "Off by default and unrelated to the metrics on this dashboard, which never leave this machine.",
      },
    ],
  },
  {
    id: "ranking",
    title: "Ranking",
    summary:
      "How search decides which files answer a question. Weights are relative to each other, not percentages.",
    icon: Scale,
    fields: [
      {
        kind: "number",
        key: "ranking.keyword",
        label: "Keyword weight",
        description: "Full-text relevance of the file's contents.",
        step: "0.1",
      },
      {
        kind: "number",
        key: "ranking.symbol",
        label: "Symbol weight",
        description: "A definition in the file whose name matches the query.",
        step: "0.1",
      },
      {
        kind: "number",
        key: "ranking.graph",
        label: "Graph weight",
        description: "How much the rest of the project depends on the file.",
        step: "0.1",
      },
      {
        kind: "number",
        key: "ranking.recency",
        label: "Recency weight",
        description: "How recently the file changed.",
        step: "0.1",
      },
      {
        kind: "number",
        key: "ranking.semantic",
        label: "Semantic weight",
        description:
          "Embedding similarity. Zero unless embeddings are on, in which case this is what turns them into ranking.",
        step: "0.1",
      },
      {
        kind: "number",
        key: "ranking.expansion_depth",
        label: "Expansion depth",
        description:
          "How far to follow the dependency graph out from a direct match. Zero disables expansion.",
        unit: "hops",
      },
      {
        kind: "number",
        key: "ranking.hop_decay",
        label: "Hop decay",
        description:
          "Score multiplier per hop away from a match. Between 0 and 1.",
        step: "0.05",
      },
      {
        kind: "number",
        key: "ranking.recency_half_life_days",
        label: "Recency half-life",
        description: "Days after which a file counts as half as recent.",
        unit: "days",
      },
    ],
  },
  {
    id: "semantic",
    title: "Embeddings",
    summary:
      "Off by default. Embedding costs index time and database size, and every deterministic path works without it.",
    icon: Eye,
    fields: [
      {
        kind: "switch",
        key: "semantic.enabled",
        label: "Compute embeddings",
        description:
          "Turning this on does not change ranking on its own — give ranking.semantic a weight as well.",
      },
      {
        kind: "select",
        key: "semantic.provider",
        label: "Provider",
        description:
          "`hashed` needs no model and no network, which is why it is the only one enabled by default.",
        options: [{ value: "hashed", label: "hashed (local, no model)" }],
      },
      {
        kind: "number",
        key: "semantic.dimensions",
        label: "Dimensions",
        description: "Vector width. Larger is more precise and larger on disk.",
      },
      {
        kind: "number",
        key: "semantic.redundancy_threshold",
        label: "Redundancy threshold",
        description:
          "Similarity at which two pieces of text count as saying the same thing. Between 0 and 1.",
        step: "0.01",
      },
      {
        kind: "number",
        key: "semantic.diversity",
        label: "Diversity",
        description:
          "How much relevance to trade for coverage when selecting results. Between 0 and 1.",
        step: "0.05",
      },
    ],
  },
  {
    id: "watch",
    title: "Watching",
    summary:
      "How the daemon notices that a project changed, and how long it waits before acting.",
    icon: Gauge,
    fields: [
      {
        kind: "switch",
        key: "watch.enabled",
        label: "Watch for changes",
        description:
          "With this off, projects are only indexed when something asks for it.",
      },
      {
        kind: "number",
        key: "watch.debounce_ms",
        label: "Debounce",
        description:
          "Quiet period before a changed file is acted on. A save that writes three times in a row should cost one index.",
        unit: "ms",
      },
      {
        kind: "number",
        key: "watch.poll_interval_ms",
        label: "Poll interval",
        description:
          "How often a project that cannot be watched is scanned instead. Network drives and some containers land here.",
        unit: "ms",
      },
    ],
  },
  {
    id: "storage",
    title: "Storage",
    summary: "Where CtxC keeps what it knows.",
    icon: Database,
    note: (
      <Notice tone="warning" title="Changing the database path starts a new database">
        The old one is left where it is, with everything in it. Nothing is
        migrated, and nothing is deleted.
      </Notice>
    ),
    fields: [
      {
        kind: "text",
        key: "storage.path",
        label: "Database path",
        description:
          "`auto` resolves to the platform data directory. Anything else is used verbatim.",
      },
    ],
  },
  {
    id: "daemon",
    title: "Daemon",
    summary:
      "The background process serving this dashboard. Changes here need a restart.",
    icon: Server,
    fields: [
      {
        kind: "switch",
        key: "daemon.enabled",
        label: "Daemon enabled",
        description: "Whether `ctxc start` will run one at all.",
      },
      {
        kind: "switch",
        key: "daemon.auto_start",
        label: "Start on demand",
        description:
          "Let commands that want a daemon start one rather than telling you to.",
      },
      {
        kind: "text",
        key: "daemon.bind",
        label: "Bind address",
        description:
          "Loopback by default. The API has a token, but it is a local API and binding it wider is a decision worth making deliberately.",
      },
      {
        kind: "number",
        key: "daemon.port",
        label: "Port",
        description:
          "Zero asks the operating system for a free one, which is what a sandboxed or ephemeral instance wants.",
      },
      {
        kind: "switch",
        key: "dashboard.enabled",
        label: "Dashboard enabled",
        description: "Whether `ctxc dashboard` has anything to open.",
      },
      {
        kind: "number",
        key: "dashboard.port",
        label: "Dashboard port",
        description: "Must differ from the daemon port.",
      },
    ],
  },
  {
    id: "metrics",
    title: "Metrics",
    summary:
      "What CtxC counts, how long it keeps it, and what a token is worth. All of it stays on this machine.",
    icon: ShieldCheck,
    fields: [
      {
        kind: "switch",
        key: "metrics.enabled",
        label: "Record metrics",
        description:
          "With this off, the Overview and Performance screens have nothing to show.",
      },
      {
        kind: "number",
        key: "metrics.raw_retention_days",
        label: "Raw event retention",
        description:
          "Days of per-operation events to keep. Zero keeps them forever. The activity feed reads these.",
        unit: "days",
      },
      {
        kind: "number",
        key: "metrics.hourly_retention_days",
        label: "Hourly aggregate retention",
        description:
          "Daily aggregates are the long-term record and are never pruned.",
        unit: "days",
      },
      {
        kind: "text",
        key: "metrics.cost_model",
        label: "Cost model",
        description:
          "The model the estimate assumes. Shown next to every cost figure so nobody reads one as a bill.",
      },
      {
        kind: "number",
        key: "metrics.cost_per_million_input_tokens",
        label: "Price per million input tokens",
        description:
          "Zero means CtxC has no rate to work from and reports no cost at all, which is the only honest answer to a question nobody has configured.",
        step: "0.01",
      },
      {
        kind: "text",
        key: "metrics.cost_currency",
        label: "Currency",
        description: "The label put in front of a cost figure.",
      },
    ],
  },
];

// -------------------------------------------------------------- the screen

export function Settings({ revision }: { revision: number }) {
  const config = useApi(() => api.config(), [revision]);

  return (
    <PageBody>
      <PageHeader
        title="Settings"
        description="Everything here is written to one TOML file. Editing that file by hand does the same thing, and this screen keeps the comments in it."
      />

      {config.error ? (
        <Failure
          message={config.error.message}
          hint={hintOf(config.error)}
          onRetry={config.reload}
        />
      ) : config.loading || !config.data ? (
        <SkeletonRows rows={5} height="h-16" />
      ) : (
        <SettingsBody view={config.data} onSaved={config.reload} />
      )}
    </PageBody>
  );
}

function SettingsBody({
  view,
  onSaved,
}: {
  view: ConfigView;
  onSaved: () => void;
}) {
  return (
    <>
      {view.restart_required ? (
        <Notice tone="warning" title="The daemon is running with older settings">
          It reads its configuration once, at startup. Restart it with{" "}
          <Code>ctxc stop</Code> then <Code>ctxc start --detach</Code> for the
          current file to take effect.
        </Notice>
      ) : null}

      {view.environment_keys.length > 0 ? (
        <Notice title="Some settings come from the environment">
          {view.environment_keys.length} key
          {view.environment_keys.length === 1 ? " is" : "s are"} set by{" "}
          <Code>CTXC_*</Code> variables, which win over the file. Those controls
          are shown but cannot be changed here.
        </Notice>
      ) : null}

      <Tabs defaultValue={GROUPS[0]!.id}>
        <TabsList className="mb-4 flex-wrap">
          {GROUPS.map((group) => (
            <TabsTrigger key={group.id} value={group.id}>
              <group.icon />
              <span className="hidden sm:inline">{group.title}</span>
            </TabsTrigger>
          ))}
          <TabsTrigger value="file">
            <Info />
            <span className="hidden sm:inline">File</span>
          </TabsTrigger>
        </TabsList>

        {GROUPS.map((group) => (
          <TabsContent key={group.id} value={group.id}>
            <GroupForm group={group} view={view} onSaved={onSaved} />
          </TabsContent>
        ))}

        <TabsContent value="file">
          <FilePanel view={view} />
        </TabsContent>
      </Tabs>
    </>
  );
}

// --------------------------------------------------------------- one group

type Draft = Record<string, string | boolean>;

function read(config: Config, key: string): string | boolean {
  const [section, name] = key.split(".") as [keyof Config, string];
  const value = (config[section] as Record<string, unknown>)[name];
  return typeof value === "boolean" ? value : String(value);
}

function initial(group: Group, config: Config): Draft {
  return Object.fromEntries(group.fields.map((field) => [field.key, read(config, field.key)]));
}

/** The environment variable a key would be overridden by. */
function envName(key: string): string {
  return `CTXC_${key.replace(".", "_").toUpperCase()}`;
}

function GroupForm({
  group,
  view,
  onSaved,
}: {
  group: Group;
  view: ConfigView;
  onSaved: () => void;
}) {
  // Keyed on the values rather than on the object, so a re-read that found the
  // same configuration does not throw away what someone is halfway through
  // typing. Only a real change replaces the form.
  const signature = JSON.stringify(initial(group, view.effective));
  const baseline = React.useMemo(() => JSON.parse(signature) as Draft, [signature]);
  const [draft, setDraft] = React.useState<Draft>(baseline);

  React.useEffect(() => setDraft(baseline), [baseline]);

  const changed = group.fields.filter(
    (field) => draft[field.key] !== baseline[field.key],
  );

  const invalid = changed.filter(
    (field) =>
      field.kind === "number" && !Number.isFinite(Number(draft[field.key])),
  );

  const save = useMutation(
    (patch: PartialConfig) => api.editConfig(patch),
    {
      onDone: (result) => {
        if (result.changed.length === 0) {
          toast.success("Nothing to save — those values were already set.");
        } else {
          toast.success(`Saved ${result.changed.length} setting${result.changed.length === 1 ? "" : "s"}.`, {
            description: result.shadowed.length
              ? `${result.shadowed.join(", ")} stays overridden by the environment.`
              : "Restart the daemon for it to run with them.",
          });
        }
        onSaved();
      },
    },
  );

  const reset = useMutation((key: string) => api.editConfig({}, [key]), {
    onDone: () => {
      toast.success("Handed back to the default.");
      onSaved();
    },
  });

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (changed.length === 0 || invalid.length > 0) return;

    const patch: PartialConfig = {};
    for (const field of changed) {
      const [section, name] = field.key.split(".") as [keyof Config, string];
      const value = draft[field.key];
      const parsed =
        field.kind === "number"
          ? Number(value)
          : field.kind === "switch"
            ? Boolean(value)
            : String(value);

      const target = (patch[section] ?? {}) as Record<string, unknown>;
      target[name] = parsed;
      (patch as Record<string, unknown>)[section] = target;
    }

    void save.run(patch);
  };

  const failure = save.error ?? reset.error;

  return (
    <form onSubmit={submit}>
      <Card>
        <CardHeader>
          <CardHeading>
            <CardTitle>{group.title}</CardTitle>
            <CardDescription>{group.summary}</CardDescription>
          </CardHeading>
        </CardHeader>

        <CardContent className="space-y-6">
          {group.note}
          {failure ? (
            <Failure message={failure.message} hint={hintOf(failure)} />
          ) : null}

          <FieldRow>
            {group.fields.map((field) => (
              <SettingControl
                key={field.key}
                field={field}
                value={draft[field.key]!}
                onChange={(value) =>
                  setDraft((current) => ({ ...current, [field.key]: value }))
                }
                inFile={view.file.keys.includes(field.key)}
                fromEnvironment={view.environment_keys.includes(field.key)}
                onReset={() => void reset.run(field.key)}
                resetting={reset.pending}
                invalid={invalid.includes(field)}
              />
            ))}
          </FieldRow>
        </CardContent>

        <CardFooter className="justify-end">
          <span className="text-muted-foreground mr-auto text-xs">
            {changed.length === 0
              ? "No unsaved changes."
              : `${changed.length} unsaved change${changed.length === 1 ? "" : "s"}.`}
          </span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={changed.length === 0 || save.pending}
            onClick={() => setDraft(baseline)}
          >
            Discard
          </Button>
          <Button
            type="submit"
            size="sm"
            disabled={changed.length === 0 || invalid.length > 0 || save.pending}
          >
            <Save />
            {save.pending ? "Saving…" : "Save changes"}
          </Button>
        </CardFooter>
      </Card>
    </form>
  );
}

function SettingControl({
  field,
  value,
  onChange,
  inFile,
  fromEnvironment,
  onReset,
  resetting,
  invalid,
}: {
  field: FieldSpec;
  value: string | boolean;
  onChange: (value: string | boolean) => void;
  inFile: boolean;
  fromEnvironment: boolean;
  onReset: () => void;
  resetting: boolean;
  invalid: boolean;
}) {
  const id = `setting-${field.key.replace(".", "-")}`;

  const hint = (
    <span className="flex items-center gap-1.5">
      {fromEnvironment ? (
        <Tooltip
          label={`Set by ${envName(field.key)}, which wins over the file. Unset the variable to edit it here.`}
        >
          <Badge variant="warning">Environment</Badge>
        </Tooltip>
      ) : inFile ? (
        <>
          <Tooltip label="Written in the configuration file.">
            <Badge variant="primary">Set</Badge>
          </Tooltip>
          <Tooltip label="Remove it from the file and use the built-in default.">
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              onClick={onReset}
              disabled={resetting}
              aria-label={`Reset ${field.label} to its default`}
            >
              <RotateCcw />
            </Button>
          </Tooltip>
        </>
      ) : (
        <Tooltip label="Not written down; this is the built-in default.">
          <Badge variant="outline">Default</Badge>
        </Tooltip>
      )}
    </span>
  );

  if (field.kind === "switch") {
    return (
      <Field label={field.label} htmlFor={id} description={field.description} hint={hint}>
        <div className="flex h-9 items-center">
          <Switch
            id={id}
            checked={Boolean(value)}
            disabled={fromEnvironment}
            onCheckedChange={onChange}
          />
        </div>
      </Field>
    );
  }

  if (field.kind === "select") {
    return (
      <Field label={field.label} htmlFor={id} description={field.description} hint={hint}>
        <Select
          value={String(value)}
          disabled={fromEnvironment}
          onValueChange={onChange}
        >
          <SelectTrigger id={id}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>
    );
  }

  return (
    <Field
      label={field.label}
      htmlFor={id}
      description={field.description}
      hint={hint}
      error={invalid ? "That is not a number." : undefined}
    >
      <div className="flex items-center gap-2">
        <Input
          id={id}
          value={String(value)}
          disabled={fromEnvironment}
          inputMode={field.kind === "number" ? "decimal" : undefined}
          step={field.kind === "number" ? field.step : undefined}
          spellCheck={false}
          aria-invalid={invalid || undefined}
          onChange={(event) => onChange(event.target.value)}
          className={field.kind === "text" ? "font-mono" : "tabular"}
        />
        {field.kind === "number" && field.unit ? (
          <span className="text-muted-foreground shrink-0 text-xs">
            {field.unit}
          </span>
        ) : null}
      </div>
    </Field>
  );
}

// ------------------------------------------------------------- the file

function FilePanel({ view }: { view: ConfigView }) {
  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardHeader>
          <CardHeading>
            <CardTitle>Where configuration comes from</CardTitle>
            <CardDescription>
              Layers are applied lowest first; each one only changes the keys it
              sets. The same list <Code>ctxc config path</Code> prints.
            </CardDescription>
          </CardHeading>
        </CardHeader>
        <CardContent>
          <ul className="divide-border/60 divide-y">
            {view.layers.map((layer, index) => (
              <li
                key={`${layer.kind}-${index}`}
                className="flex flex-wrap items-center gap-3 py-2 text-sm"
              >
                <Badge variant={layer.applied ? "primary" : "outline"}>
                  {layer.kind}
                </Badge>
                <span className="text-muted-foreground min-w-0 flex-1 truncate font-mono text-xs">
                  {layer.path ?? "—"}
                </span>
                {layer.applied ? null : (
                  <span className="text-muted-foreground text-xs">
                    not present
                  </span>
                )}
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardHeading>
            <CardTitle className="font-mono text-xs break-all">
              {view.file.path}
            </CardTitle>
            <CardDescription>
              What is actually written down. Everything else is a built-in
              default.
            </CardDescription>
          </CardHeading>
          <Badge variant={view.file.exists ? "primary" : "outline"}>
            {view.file.exists ? `${view.file.keys.length} keys` : "not created yet"}
          </Badge>
        </CardHeader>
        <CardContent>
          <pre className="bg-muted/40 max-h-80 overflow-auto rounded-md border p-3 font-mono text-xs leading-relaxed">
            {view.file.toml.trim() || "# nothing set; every value is a default"}
          </pre>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardHeading>
            <CardTitle>Effective configuration</CardTitle>
            <CardDescription>
              Every layer merged, as <Code>ctxc config show</Code> prints it.
            </CardDescription>
          </CardHeading>
        </CardHeader>
        <CardContent>
          <pre className="bg-muted/40 max-h-96 overflow-auto rounded-md border p-3 font-mono text-xs leading-relaxed">
            {view.toml}
          </pre>
        </CardContent>
      </Card>
    </div>
  );
}
