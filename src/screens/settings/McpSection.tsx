import { useEffect } from "react";
import { Check, Copy, Info, KeyRound, Plug } from "lucide-react";
import { cx } from "../../lib/cx";
import { Button, Icon, Kbd, Toggle } from "../../design-system/atoms";
import { ListCard, ListRow, Notice, SectionHeader, SegmentedControl } from "../../design-system/molecules";
import { useAgents, type Copied } from "../../state/agents";
import { useScreenKeys } from "../../state/useScreenKeys";
import { agentTabs, howTo, lastUsedText, masked, shown } from "./mcpConnection";
import shared from "./settings.module.css";
import styles from "./McpSection.module.css";

function CopyButton({ what, text, label, shortcut }: { what: Copied; text: string; label: string; shortcut?: string }) {
  const { copied, copy } = useAgents();
  const done = copied === what;
  return (
    <Button variant="mini" icon={done ? Check : Copy} shortcut={shortcut} aria-label={done ? `Copied ${label}` : `Copy ${label}`} onClick={() => void copy(what, text)}>
      {done ? "Copied" : "Copy"}
    </Button>
  );
}

/**
 * Settings → AI → Your own agents: turns on Moss's MCP server, so agents like Claude Code
 * read, manage and send your mail, and shows what to paste into each one.
 */
export function McpSection({ now = Date.now }: { now?: () => number }) {
  const { view, agent, asking, problem, load, setOn, newToken, choose, copy } = useAgents();

  useEffect(() => {
    void load();
  }, [load]);

  const on = !!view?.on && !!view.url && !!view.token;
  const connection = on ? { url: view!.url!, token: view!.token!, bridge: view!.bridge } : undefined;
  const how = connection && howTo(agent, connection);

  // A: on or off. While on: 1 – 5 pick the agent, C copies, N makes a new token (twice).
  useScreenKeys((k, e) => {
    if (!view || k.typing || k.mod || k.alt) return;
    if (k.key === "a") return void setOn(!view.on), true;
    if (!connection || !how) return;
    if (k.key === "escape" && asking) {
      useAgents.setState({ asking: false });
      e.stopImmediatePropagation();
      return true;
    }
    const tab = k.digit ? agentTabs[k.digit - 1] : undefined;
    if (tab) return choose(tab.value), true;
    if (k.key === "c") return void copy(agent, agent === "other" ? connection.url : how.text), true;
    if (k.key === "n") return void newToken(), true;
  });

  if (!view) return null;
  const label = agentTabs.find((t) => t.value === agent)!.label;

  return (
    <div className={shared.group}>
      <SectionHeader label="Your own agents (MCP)" />
      <ListCard aria-label="Your own agents">
        <ListRow className={styles.row}>
          <span className={styles.tile}>
            <Icon icon={Plug} size={16} tone="ink-2" />
          </span>
          <span className={shared.text}>
            <span className={shared.name}>Connect your agents</span>
            <span className={cx(shared.sub, styles.wrap)}>Let Claude Code, Codex, Cursor and others manage your mail through MCP: sort it, answer it, send it.</span>
          </span>
          <Toggle on={view.on} label="Connect your agents" onChange={(next) => void setOn(next)} />
        </ListRow>
        {connection && how && (
          <div className={styles.connect}>
            <div className={styles.tabs}>
              <SegmentedControl label="Agent" value={agent} options={agentTabs} onChange={choose} size="sm" />
              <Kbd>1 – 5</Kbd>
            </div>
            <p className={styles.where}>{how.where}</p>
            {agent === "other" ? (
              <dl className={styles.fields}>
                <div className={styles.field}>
                  <dt>URL</dt>
                  <dd className={styles.code}>{connection.url}</dd>
                  <CopyButton what="url" text={connection.url} label="URL" shortcut="C" />
                </div>
                <div className={styles.field}>
                  <dt>Token</dt>
                  <dd className={styles.code}>{masked(connection.token)}</dd>
                  <CopyButton what="token" text={connection.token} label="token" />
                </div>
              </dl>
            ) : (
              <div className={styles.snippet}>
                <pre className={styles.code} aria-label={`Connection for ${label}`}>
                  {shown(how.text, connection.token)}
                </pre>
                <CopyButton what={agent} text={how.text} label={`for ${label}`} shortcut="C" />
              </div>
            )}
            <div className={styles.foot}>
              <span className={cx(styles.used, !!view.lastUsed && now() - view.lastUsed < 120_000 && styles.live)}>
                <span className={styles.dot} aria-hidden />
                {lastUsedText(view.lastUsed, now())}
              </span>
              <Button variant="secondary" icon={KeyRound} shortcut="N" aria-label={asking ? "Yes, make a new token" : "New token"} onClick={() => void newToken()}>
                {asking ? "Yes, new token" : "New token"}
              </Button>
            </div>
            {asking && <p className={shared.small}>Agents using the old token stop working until you paste the new one.</p>}
          </div>
        )}
      </ListCard>
      {problem && <p className={cx(shared.small, styles.problem)}>{problem}</p>}
      <Notice variant="footnote" icon={Info}>
        Agents you connect can read, manage and send your email. A cloud agent sends what it reads to its company.
      </Notice>
    </div>
  );
}
