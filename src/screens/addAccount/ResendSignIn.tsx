import { useId, useState } from "react";
import { CircleCheck } from "lucide-react";
import { providerById } from "../../data/providers";
import { Button, Icon, TextInput } from "../../design-system/atoms";
import { Field, ProviderKind, ScreenHeading, SectionHeader, ShortcutHint } from "../../design-system/molecules";
import { DomainList } from "../../design-system/organisms";
import { FlowLayout } from "../../design-system/templates";
import { useAddAccount } from "../../state/addAccount";
import styles from "./AddAccount.module.css";
import { Footnote, SetupProblem } from "./parts";

/** Shows a key's prefix and last three characters only, e.g. re_••••7Qx. */
export function maskKey(key: string) {
  if (key.length <= 6) return key;
  return `${key.slice(0, 3)}${"•".repeat(Math.max(0, key.length - 6))}${key.slice(-3)}`;
}

/** Resend: an API key, then which domains to receive mail for. */
export function ResendSignIn() {
  const f = useAddAccount();
  const { letter, name } = providerById("resend");
  const keyId = useId();
  const [editing, setEditing] = useState(false);
  const { status, domains, key, cursor } = f.resend;
  const chosen = domains.filter((d) => d.chosen).length;

  return (
    <FlowLayout gap={24}>
      <ProviderKind letter={letter} name={name} />
      <ScreenHeading title="Connect Resend" sub="Paste an API key with Full access, then pick which domains Moss should receive mail for." />
      <Field
        label="API key"
        htmlFor={keyId}
        help="Full access, all domains · resend.com/api-keys"
        note={
          status === "works"
            ? `Found ${domains.length} ${domains.length === 1 ? "domain" : "domains"} on this key.`
            : status === "bad"
              ? f.resend.problem
              : undefined
        }
        noteTone={status === "bad" ? "error" : "ok"}
      >
        <TextInput
          id={keyId}
          data-paste-target
          autoComplete="off"
          placeholder="re_…"
          invalid={status === "bad"}
          value={editing ? key : maskKey(key)}
          onFocus={() => setEditing(true)}
          onBlur={() => setEditing(false)}
          onChange={(e) => void f.checkResendKey(e.target.value)}
          trailing={
            status === "works" && (
              <span className={styles.keyWorks}>
                <Icon icon={CircleCheck} size={16} tone="ai" />
                Key works
              </span>
            )
          }
        />
      </Field>
      {status === "works" && (
        <>
          <div className={styles.listHead}>
            <SectionHeader
              label="Receive mail for"
              right={
                <span className={styles.count}>
                  {chosen} of {domains.length} chosen
                </span>
              }
            />
          </div>
          <DomainList
            domains={domains}
            cursor={cursor}
            onToggle={f.toggleDomain}
            onAddresses={f.setDomainAddresses}
            onCursor={(i) => useAddAccount.setState((s) => ({ resend: { ...s.resend, cursor: i } }))}
          />
        </>
      )}
      <SetupProblem />
      <div className={styles.actions}>
        <Button variant="primary" size="lg" shortcut="↵" disabled={!chosen} busy={f.busy} onClick={() => void f.connect()}>
          {f.busy ? "Connecting…" : chosen ? `Connect ${chosen} ${chosen === 1 ? "domain" : "domains"}` : "Connect"}
        </Button>
        {status === "works" && <ShortcutHint keys="Space" label="Choose or skip a domain" />}
      </div>
      <Footnote>Your API key is stored in the macOS Keychain. Moss uses it only to fetch and send mail for these domains.</Footnote>
    </FlowLayout>
  );
}
