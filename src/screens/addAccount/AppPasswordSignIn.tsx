import { useId } from "react";
import { Building2 } from "lucide-react";
import { appPasswordCopy, needsICloudLogin, providerById } from "../../data/providers";
import { Button, TextInput } from "../../design-system/atoms";
import { Field, ScreenHeading, Notice, ProviderKind } from "../../design-system/molecules";
import { SetupSteps } from "../../design-system/organisms";
import { FlowLayout } from "../../design-system/templates";
import { useAddAccount } from "../../state/addAccount";
import styles from "./AddAccount.module.css";
import { AdvancedLink, Footnote, SecretInput, SetupProblem } from "./parts";
import { openLink } from "../../lib/links";

/** Gmail, iCloud, Yahoo and Fastmail: an app password made on the provider's website. */
export function AppPasswordSignIn({ provider }: { provider: keyof typeof appPasswordCopy }) {
  const f = useAddAccount();
  const copy = appPasswordCopy[provider];
  const { letter, name } = providerById(provider);
  const emailId = useId();
  const passwordId = useId();
  const loginId = useId();
  const failed = f.error === "rejected";
  const gmail = provider === "gmail";

  return (
    <FlowLayout gap={24}>
      <ProviderKind letter={letter} name={name} />
      <ScreenHeading title={failed ? copy.error.title : copy.title} sub={failed ? copy.error.sub : copy.sub} />
      {!failed && <SetupSteps steps={copy.steps} done={f.stepsDone} onOpen={f.openStep} />}
      <Field label={copy.emailLabel} htmlFor={emailId}>
        <TextInput id={emailId} type="email" value={f.email} onChange={(e) => f.setField("email", e.target.value)} />
      </Field>
      {copy.login && needsICloudLogin(f.email) && (
        <Field label={copy.login.label} htmlFor={loginId} help={copy.login.help}>
          <TextInput
            id={loginId}
            type="email"
            placeholder={copy.login.placeholder}
            value={f.login}
            onChange={(e) => f.setField("login", e.target.value)}
          />
        </Field>
      )}
      <Field label={copy.passwordLabel} htmlFor={passwordId} note={failed ? copy.error.note : undefined} noteTone="error">
        <SecretInput
          id={passwordId}
          data-paste-target
          placeholder={copy.placeholder}
          invalid={failed}
          reveal={gmail && !failed}
          paste
          value={f.password}
          onChange={(e) => f.setField("password", e.target.value)}
        />
      </Field>
      {failed && (
        <SetupSteps
          label="How to fix it"
          steps={[copy.error.fix, { title: "Paste it above and try again" }]}
          done={0}
          onOpen={(_, url) => openLink(url)}
        />
      )}
      <SetupProblem />
      <div className={styles.actions}>
        <Button variant="primary" size="lg" shortcut="↵" busy={f.busy} onClick={() => void f.connect()}>
          {f.busy ? "Connecting…" : failed ? "Try again" : "Connect"}
        </Button>
        {gmail && (
          <>
            <span className={styles.spacer} />
            <AdvancedLink label="Use your own Google OAuth client" onClick={f.toggleAdvanced} />
          </>
        )}
      </div>
      {gmail && failed && (
        <Notice variant="note" icon={Building2}>
          No App passwords page at Google? Your admin turned them off. Use your own Google OAuth client instead.
        </Notice>
      )}
      <Footnote>{failed ? "Nothing was saved. Moss only stores a password once it works." : copy.footnote}</Footnote>
    </FlowLayout>
  );
}
