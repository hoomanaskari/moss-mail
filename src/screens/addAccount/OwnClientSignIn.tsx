import { useId } from "react";
import { Building2, ClockAlert } from "lucide-react";
import { googleOAuthSteps, microsoftClientSteps, providerById } from "../../data/providers";
import { Button, TextInput } from "../../design-system/atoms";
import { Field, ScreenHeading, Notice, ProviderKind } from "../../design-system/molecules";
import { SetupSteps } from "../../design-system/organisms";
import { FlowLayout } from "../../design-system/templates";
import { useAddAccount } from "../../state/addAccount";
import styles from "./AddAccount.module.css";
import { AdvancedLink, Footnote, SecretInput, SetupProblem } from "./parts";

const copy = {
  gmail: {
    title: "Use your own Google OAuth client",
    sub: "Sign in through your own Google Cloud project instead of an app password. It takes about 10 minutes.",
    caution: "In “Testing” mode, Google signs you out every 7 days. Publish the app in step 3 to stay signed in.",
    steps: googleOAuthSteps,
    button: "Sign in with Google",
    back: "Use an app password instead",
    footnote: "Your client secret and sign-in token stay in the macOS Keychain.",
  },
  outlook: {
    title: "Use your own Microsoft client ID",
    sub: "Most people don’t need this. Use it if your IT admin asks you to. It takes about 5 minutes.",
    caution: "Pick “personal and work accounts” in step 2. Otherwise Outlook.com and Hotmail can’t sign in.",
    steps: microsoftClientSteps,
    button: "Sign in with Microsoft",
    back: "Use Moss’s built-in sign-in",
    footnote: "Your sign-in token stays in the macOS Keychain. A client ID isn’t secret, so it’s fine to share.",
  },
};

/** The advanced way: sign in through an app you register yourself with Google or Microsoft. */
export function OwnClientSignIn({ provider }: { provider: "gmail" | "outlook" }) {
  const f = useAddAccount();
  const c = copy[provider];
  const { letter, name } = providerById(provider);
  const idField = useId();
  const secretField = useId();

  return (
    <FlowLayout gap={22} tight>
      <ProviderKind letter={letter} name={name} extra="Advanced" />
      <ScreenHeading title={c.title} sub={c.sub} />
      <Notice variant="caution" icon={ClockAlert}>
        {c.caution}
      </Notice>
      <SetupSteps steps={c.steps} done={f.stepsDone} onOpen={f.openStep} />
      {provider === "gmail" ? (
        <div className={styles.pair}>
          <Field label="Client ID" htmlFor={idField}>
            <TextInput
              id={idField}
              data-paste-target
              placeholder="…apps.googleusercontent.com"
              value={f.clientId}
              onChange={(e) => f.setField("clientId", e.target.value)}
            />
          </Field>
          <Field label="Client secret" htmlFor={secretField}>
            <SecretInput
              id={secretField}
              paste
              placeholder="GOCSPX-…"
              value={f.clientSecret}
              onChange={(e) => f.setField("clientSecret", e.target.value)}
            />
          </Field>
        </div>
      ) : (
        <Field label="Application (client) ID" htmlFor={idField}>
          <TextInput
            id={idField}
            data-paste-target
            placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
            value={f.clientId}
            onChange={(e) => f.setField("clientId", e.target.value)}
          />
        </Field>
      )}
      <SetupProblem />
      <div className={styles.actions}>
        <Button variant="primary" size="lg" shortcut="↵" disabled={f.busy} onClick={() => void f.connect()}>
          {c.button}
        </Button>
        <span className={styles.spacer} />
        <AdvancedLink prefix={false} label={c.back} onClick={f.toggleAdvanced} />
      </div>
      {provider === "gmail" && (
        <Notice variant="note" icon={Building2}>
          Google will say “Google hasn’t verified this app”. That’s normal for your own app: click Advanced → Go to Moss.
        </Notice>
      )}
      <Footnote>{c.footnote}</Footnote>
    </FlowLayout>
  );
}
