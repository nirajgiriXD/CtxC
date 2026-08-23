/**
 * Everything `ctxc` can be asked to do.
 *
 * The list comes from the daemon, which built it from the same `clap`
 * definition that parses the command line — so this page cannot describe a flag
 * the binary does not have, and cannot miss one it does.
 *
 * Where a command has an equivalent in the dashboard, the card links straight
 * to it. Where it has not, the card says so plainly. A commands page that
 * implied everything had a button would be worse than one that admits which
 * work belongs in a terminal.
 */

import * as React from "react";
import {
  ArrowRight,
  Check,
  Copy,
  Terminal as TerminalIcon,
  TriangleAlert,
} from "lucide-react";

import { api, type CommandInfo, type OptionInfo } from "../lib/api";
import { href, navigate, ROUTES, type Route } from "../lib/router";
import { cn } from "../lib/utils";
import { hintOf, useApi } from "../lib/useApi";
import { PageBody, PageHeader, Section } from "../components/page";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardHeading,
  CardTitle,
  Empty,
  Failure,
  Input,
  Notice,
  SkeletonRows,
  Tooltip,
} from "../components/ui";

export function Commands() {
  const catalog = useApi(() => api.commands(), []);
  const [filter, setFilter] = React.useState("");

  const commands = React.useMemo(() => {
    if (!catalog.data) return [];
    const needle = filter.trim().toLowerCase();
    if (!needle) return catalog.data.commands;

    // Matching against a whole subtree keeps `project` visible when someone
    // types "pause": the parent is how they get to the subcommand.
    const matches = (command: CommandInfo): boolean =>
      [command.path, command.summary, command.description]
        .filter(Boolean)
        .some((text) => text!.toLowerCase().includes(needle)) ||
      (command.subcommands ?? []).some(matches);

    return catalog.data.commands.filter(matches);
  }, [catalog.data, filter]);

  return (
    <PageBody>
      <PageHeader
        title="Commands"
        description={
          catalog.data
            ? `Every command ${catalog.data.name} ${catalog.data.version} accepts, read from the binary that is running.`
            : "Every command the CLI accepts."
        }
        actions={
          <Input
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="Filter commands…"
            className="w-56"
            aria-label="Filter commands"
          />
        }
      />

      {catalog.error ? (
        <Failure
          message={catalog.error.message}
          hint={hintOf(catalog.error)}
          onRetry={catalog.reload}
        />
      ) : catalog.loading ? (
        <SkeletonRows rows={6} height="h-24" />
      ) : !catalog.data ? null : (
        <>
          <Notice title="The CLI and this dashboard are two ways into the same daemon">
            Both talk to the routes listed under System, and neither has a
            privileged path around the other. Anything you do here you can script,
            and anything you script you can watch here.
          </Notice>

          {commands.length === 0 ? (
            <Card>
              <Empty
                icon={TerminalIcon}
                title="No command matches that."
                hint="Try part of a name, like `project` or `metrics`."
              />
            </Card>
          ) : (
            <div className="grid gap-4 lg:grid-cols-2">
              {commands.map((command) => (
                <CommandCard key={command.path} command={command} />
              ))}
            </div>
          )}

          <Section
            title="Global options"
            description="Accepted by every command above."
          >
            <Card>
              <CardContent className="px-5 py-4">
                <OptionTable options={catalog.data.global_options} />
              </CardContent>
            </Card>
          </Section>
        </>
      )}
    </PageBody>
  );
}

function CommandCard({ command }: { command: CommandInfo }) {
  return (
    // `min-w-0`, because an example command line is one long unbreakable string
    // and a grid item is otherwise as wide as its widest content.
    <Card className="min-w-0">
      <CardHeader>
        <CardHeading>
          <CardTitle className="font-mono">ctxc {command.path}</CardTitle>
          {command.summary ? (
            <CardDescription>{command.summary}</CardDescription>
          ) : null}
        </CardHeading>
        <Equivalent command={command} />
      </CardHeader>

      <CardContent className="space-y-4">
        {command.description ? (
          <p className="text-muted-foreground text-xs leading-relaxed whitespace-pre-line">
            {command.description}
          </p>
        ) : null}

        <Usage usage={command.usage} />

        {command.arguments && command.arguments.length > 0 ? (
          <div className="space-y-1.5">
            <p className="text-xs font-medium">Arguments</p>
            <ul className="space-y-1">
              {command.arguments.map((argument) => (
                <li key={argument.name} className="flex flex-wrap gap-2 text-xs">
                  <span className="font-mono">
                    {argument.required ? `<${argument.name}>` : `[${argument.name}]`}
                    {argument.repeated ? "…" : ""}
                  </span>
                  <span className="text-muted-foreground min-w-0 flex-1">
                    {argument.help}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {command.options && command.options.length > 0 ? (
          <div className="space-y-1.5">
            <p className="text-xs font-medium">Options</p>
            <OptionTable options={command.options} />
          </div>
        ) : null}

        {command.subcommands && command.subcommands.length > 0 ? (
          <div className="space-y-2">
            <p className="text-xs font-medium">Subcommands</p>
            <ul className="divide-border/60 divide-y">
              {command.subcommands.map((child) => (
                <li key={child.path} className="space-y-1 py-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-mono text-xs">ctxc {child.path}</span>
                    <Equivalent command={child} compact />
                  </div>
                  {child.summary ? (
                    <p className="text-muted-foreground text-xs">
                      {child.summary}
                    </p>
                  ) : null}
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {command.examples && command.examples.length > 0 ? (
          <div className="space-y-1.5">
            <p className="text-xs font-medium">Examples</p>
            {command.examples.map((example) => (
              <Usage key={example} usage={example} />
            ))}
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

/** Where this command's work is done in the dashboard, if it is. */
function Equivalent({
  command,
  compact,
}: {
  command: CommandInfo;
  compact?: boolean;
}) {
  const target = command.dashboard;

  // A route the daemon named that this build of the dashboard does not have
  // would be a link to nowhere. Treating it as "no equivalent" is the honest
  // reading, and it can only happen if the two halves are different versions.
  const route =
    target && ROUTES.includes(target.route as Route)
      ? (target.route as Route)
      : undefined;

  if (!target || !route) {
    return compact ? null : (
      <Tooltip label="This one produces a stream, wraps a process, or rebuilds the binary — a terminal is the right place for it.">
        <Badge variant="outline">
          <TriangleAlert /> Terminal only
        </Badge>
      </Tooltip>
    );
  }

  return (
    <Button
      variant={compact ? "ghost" : "outline"}
      size="sm"
      onClick={() => navigate(href(route))}
    >
      {target.label} <ArrowRight />
    </Button>
  );
}

/** A command line, with a button that puts it on the clipboard. */
function Usage({ usage }: { usage: string }) {
  const [copied, setCopied] = React.useState(false);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(usage);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access can be refused; the text is on screen either way, so
      // there is nothing worth interrupting the reader about.
    }
  };

  return (
    <div className="bg-muted/50 flex min-w-0 items-center gap-2 rounded-md border px-3 py-2">
      <code className="min-w-0 flex-1 overflow-x-auto font-mono text-xs whitespace-pre">
        {usage}
      </code>
      <Button
        variant="ghost"
        size="icon-sm"
        onClick={copy}
        aria-label={`Copy: ${usage}`}
        className={cn(copied && "text-success")}
      >
        {copied ? <Check /> : <Copy />}
      </Button>
    </div>
  );
}

function OptionTable({ options }: { options: OptionInfo[] }) {
  if (options.length === 0) {
    return <p className="text-muted-foreground text-xs">None.</p>;
  }

  return (
    <ul className="space-y-1.5">
      {options.map((option) => (
        <li key={option.name} className="flex flex-wrap gap-x-3 gap-y-1 text-xs">
          <span className="font-mono">
            {option.short ? `-${option.short}, ` : ""}
            --{option.name}
            {option.value_name ? ` <${option.value_name}>` : ""}
          </span>
          <span className="text-muted-foreground min-w-0 flex-1">
            {option.help}
          </span>
          {option.default ? (
            <Badge variant="outline">default {option.default}</Badge>
          ) : null}
          {option.values && option.values.length > 0 ? (
            <Badge variant="outline">{option.values.join(" | ")}</Badge>
          ) : null}
        </li>
      ))}
    </ul>
  );
}
