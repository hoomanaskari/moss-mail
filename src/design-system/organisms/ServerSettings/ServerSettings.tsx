import { CircleAlert, CircleCheck } from "lucide-react";
import type { Security, Server, ServerSettings as Servers } from "../../../services/accountSetup";
import { Icon, Select, TextInput } from "../../atoms";
import styles from "./ServerSettings.module.css";

export interface ServerSettingsProps {
  /** True when Moss found them; false asks you to type them. */
  found: boolean;
  servers: Servers;
  /** The email's domain, for the usual server names. */
  domain: string;
  onChange: (which: keyof Servers, patch: Partial<Server>) => void;
}

const security = [
  { value: "ssl", label: "SSL/TLS" },
  { value: "starttls", label: "STARTTLS" },
  { value: "none", label: "None" },
];

const rows = [
  { which: "incoming", label: "Incoming · IMAP", prefix: "imap" },
  { which: "outgoing", label: "Outgoing · SMTP", prefix: "smtp" },
] as const;

/** Incoming and outgoing mail servers for an IMAP account. */
export function ServerSettings({ found, servers, domain, onChange }: ServerSettingsProps) {
  return (
    <section className={styles.card} aria-label="Server settings">
      <div className={styles.head}>
        <Icon icon={found ? CircleCheck : CircleAlert} size={18} tone={found ? "ai" : "due"} />
        <span className={styles.title}>{found ? "Server settings found" : "Type your server settings"}</span>
        <span className={styles.source}>
          {found ? "from Mozilla’s list of email providers" : `Search “${domain} IMAP settings”, or ask whoever runs your email`}
        </span>
        {found && <span className={styles.hint}>Change only if your provider says so</span>}
      </div>
      <div className={styles.columns} aria-hidden>
        <span className={styles.pad} />
        <span className={styles.server}>Server</span>
        <span className={styles.port}>Port</span>
        <span className={styles.security}>Security</span>
      </div>
      {rows.map(({ which, label, prefix }) => (
        <div key={which} className={styles.row}>
          <span className={styles.pad}>{label}</span>
          <TextInput
            compact
            className={styles.server}
            aria-label={`${label} server`}
            placeholder={`${prefix}.${domain}`}
            value={servers[which].host}
            onChange={(e) => onChange(which, { host: e.target.value })}
          />
          <TextInput
            compact
            className={styles.port}
            aria-label={`${label} port`}
            inputMode="numeric"
            value={servers[which].port}
            onChange={(e) => onChange(which, { port: e.target.value })}
          />
          <Select
            variant="field"
            className={styles.security}
            aria-label={`${label} security`}
            value={servers[which].security}
            options={security}
            onChange={(v) => onChange(which, { security: v as Security })}
          />
        </div>
      ))}
    </section>
  );
}
