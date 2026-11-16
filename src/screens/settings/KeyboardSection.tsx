import { Kbd, SectionLabel } from "../../design-system/atoms";
import { ScreenHeading } from "../../design-system/molecules";
import shared from "./settings.module.css";
import styles from "./KeyboardSection.module.css";

type Group = { title: string; keys: [action: string, ...keys: string[]][] };

const left: Group[] = [
  {
    title: "Move around",
    keys: [
      ["Next or previous email", "↓", "↑"],
      ["Open", "↵"],
      ["Go back", "Esc"],
      ["Next split", "Tab"],
      ["All accounts", "⌘0"],
      ["One account", "⌘1–⌘9"],
      ["Search", "/"],
      ["Done list", "D"],
      ["Inbox zero", "Z"],
      ["Blocked mail", "⌘⇧B"],
    ],
  },
  {
    title: "Ask AI",
    keys: [
      ["Ask AI or run a command", "⌘K"],
      ["Summarise the open email", "S"],
      ["Choose a kind of summary first", "⇧S"],
    ],
  },
];

const right: Group[] = [
  {
    title: "Act on email",
    keys: [
      ["Done, or not done again", "E"],
      ["Delete (to Trash)", "⌫"],
      ["Delete a search result", "⌘⌫"],
      ["Remind me", "H"],
      ["Remind me if no reply", "⇧H"],
      ["Move to Important, or out of it", "M"],
      ["Reply", "R"],
      ["Reply all", "⇧R"],
      ["Forward", "F"],
      ["New email", "C"],
      ["Send", "⌘↵"],
      ["Send later", "⌘⇧↵"],
      ["Attach files", "⌘⇧A"],
      ["Remove an attached file", "⌫"],
      ["Block sender", "B"],
      ["Block their domain", "⇧B"],
      ["Accept sender", "A"],
      ["Accept their domain", "⇧A"],
      ["Unblock sender", "U"],
      ["Unread, or read again", "U"],
    ],
  },
  {
    title: "App",
    keys: [
      ["Settings", "⌘,"],
      ["Add account", "⌘N"],
      ["Undo done, delete or snooze", "⌘Z"],
    ],
  },
];

function Column({ groups }: { groups: Group[] }) {
  return (
    <div className={styles.column}>
      {groups.map((g) => (
        <section key={g.title} className={styles.group} aria-label={g.title}>
          <SectionLabel as="h3">{g.title}</SectionLabel>
          <dl className={styles.list}>
            {g.keys.map(([action, ...keys]) => (
              <div key={action} className={styles.row}>
                <dt className={styles.action}>{action}</dt>
                {keys.map((k) => (
                  <dd key={k} className={styles.key}>
                    <Kbd>{k}</Kbd>
                  </dd>
                ))}
              </div>
            ))}
          </dl>
        </section>
      ))}
    </div>
  );
}

/** Settings → Keyboard shortcuts: every key in one place. Also opens with ? from anywhere. */
export function KeyboardSection() {
  return (
    <div className={shared.page}>
      <ScreenHeading title="Keyboard shortcuts" sub="Every action in Moss has a key. Press ? anywhere to see this list." />
      <div className={styles.columns}>
        <Column groups={left} />
        <Column groups={right} />
      </div>
    </div>
  );
}
