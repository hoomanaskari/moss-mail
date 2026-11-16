import { useId } from "react";
import { providerById } from "../../data/providers";
import { Button, TextInput } from "../../design-system/atoms";
import { Field, ScreenHeading, ProviderKind } from "../../design-system/molecules";
import { ServerSettings } from "../../design-system/organisms";
import { FlowLayout } from "../../design-system/templates";
import { useAddAccount } from "../../state/addAccount";
import styles from "./AddAccount.module.css";
import { Footnote, SecretInput, SetupProblem } from "./parts";

/** Any other email: address and password, with the servers found or typed in. */
export function ImapSignIn() {
  const f = useAddAccount();
  const { name } = providerById("imap");
  const emailId = useId();
  const passwordId = useId();
  const domain = f.email.split("@")[1] ?? "";
  const missing = f.imap.status === "not-found";

  return (
    <FlowLayout gap={24}>
      <ProviderKind name={name} />
      <ScreenHeading
        title={missing ? "We couldn’t find your server settings" : "Connect your email"}
        sub={
          missing
            ? `Moss looked for ${domain} in Mozilla’s list of email providers but it isn’t there. Type the settings below.`
            : "Type your email and password. Moss finds the server settings for you."
        }
      />
      <Field label="Email" htmlFor={emailId}>
        <TextInput
          id={emailId}
          type="email"
          value={f.email}
          onChange={(e) => f.setField("email", e.target.value)}
          onBlur={(e) => void f.lookUpServers(e.target.value)}
        />
      </Field>
      <Field label="Password" htmlFor={passwordId}>
        <SecretInput id={passwordId} data-paste-target reveal value={f.password} onChange={(e) => f.setField("password", e.target.value)} />
      </Field>
      {f.imap.status !== "idle" && (
        <ServerSettings found={!missing} servers={f.imap.servers} domain={domain} onChange={f.setServer} />
      )}
      <SetupProblem />
      <div className={styles.actions}>
        <Button variant="primary" size="lg" shortcut="↵" busy={f.busy} onClick={() => void f.connect()}>
          {f.busy ? "Connecting…" : "Connect"}
        </Button>
      </div>
      <Footnote>Moss saves your password in the macOS Keychain, never in a plain file.</Footnote>
    </FlowLayout>
  );
}
