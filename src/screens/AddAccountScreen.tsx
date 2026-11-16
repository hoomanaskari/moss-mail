import { appPasswordCopy, googleOAuthSteps, microsoftClientSteps, type SetupStepCopy } from "../data/providers";
import { PageTitle } from "../design-system/atoms";
import { FlowSteps, ShortcutHint } from "../design-system/molecules";
import { ShortcutBar, TopBar, type ShortcutBarHint } from "../design-system/organisms";
import { AppFrame } from "../design-system/templates";
import { accountSetup } from "../services/accountSetup";
import { useAddAccount } from "../state/addAccount";
import { useScreenKeys, type KeyInfo } from "../state/useScreenKeys";
import styles from "./AddAccountScreen.module.css";
import { AppPasswordSignIn } from "./addAccount/AppPasswordSignIn";
import { ChooseProvider } from "./addAccount/ChooseProvider";
import { ImapSignIn } from "./addAccount/ImapSignIn";
import { copyText, openInBrowser, OutlookSignIn } from "./addAccount/OutlookSignIn";
import { OwnClientSignIn } from "./addAccount/OwnClientSignIn";
import { ResendSignIn } from "./addAccount/ResendSignIn";
import { Syncing } from "./addAccount/Syncing";
import { AiSetup } from "./addAccount/AiSetup";
import { useAiSettings } from "../state/aiSettings";

type Flow = ReturnType<typeof useAddAccount.getState>;

const isAppPassword = (p?: string): p is keyof typeof appPasswordCopy => !!p && p in appPasswordCopy;

/** The setup steps on the sign-in page, so ⌘1–⌘4 and ⌘O can open their pages. */
function stepsOnScreen(f: Flow): SetupStepCopy[] {
  if (f.advanced) return f.provider === "gmail" ? googleOAuthSteps : microsoftClientSteps;
  if (isAppPassword(f.provider)) {
    const copy = appPasswordCopy[f.provider];
    return f.error ? [copy.error.fix] : copy.steps;
  }
  return [];
}

/** What the footer reminds you of, for the step on screen. */
function hints(f: Flow): ShortcutBarHint[] {
  const esc = { keys: "Esc", label: "Back" };
  if (f.stage === "provider") {
    return [
      { keys: "1 – 7", label: "Pick" },
      { keys: "↵", label: "Continue" },
      { keys: "Esc", label: "Cancel" },
    ];
  }
  if (f.stage === "ai") {
    return [
      { keys: "↑ ↓", label: "Choose" },
      { keys: "↵", label: "Download" },
      { keys: "S", label: "Skip" },
    ];
  }
  if (f.stage === "sync") {
    return [
      { keys: "↵", label: "Go to inbox" },
      { keys: "← →", label: "Account colour" },
      { keys: "A", label: "Add another account" },
    ];
  }
  if (f.advanced) {
    return [
      { keys: "⌘1 – ⌘4", label: "Open each step" },
      { keys: "⌘V", label: "Paste" },
      { keys: "↵", label: "Sign in" },
      { keys: "⌥A", label: f.provider === "gmail" ? "App password" : "Built-in sign-in" },
      esc,
    ];
  }
  if (f.provider === "outlook") {
    return f.error
      ? [{ keys: "⌘C", label: "Copy approval link" }, { keys: "↵", label: "Try again" }, { keys: "⌥A", label: "Advanced" }, esc]
      : [{ keys: "⌘O", label: "Open browser again" }, { keys: "⌘C", label: "Copy sign-in link" }, { keys: "⌥A", label: "Advanced" }, esc];
  }
  if (f.provider === "resend") {
    return [
      { keys: "Space", label: "Choose" },
      { keys: "⌥↓", label: "Pick addresses" },
      { keys: "↵", label: "Connect" },
      esc,
    ];
  }
  if (f.provider === "imap") {
    return [{ keys: "Tab", label: "Next field" }, { keys: "⌘E", label: "Edit server settings" }, { keys: "↵", label: "Connect" }, esc];
  }
  const copy = appPasswordCopy[f.provider as keyof typeof appPasswordCopy];
  const steps = stepsOnScreen(f);
  const open = steps.length > 1 ? `⌘${Math.min(f.stepsDone, steps.length - 2) + 1}` : "⌘O";
  return [
    { keys: open, label: copy.openLabel },
    { keys: "⌘V", label: "Paste" },
    { keys: "↵", label: f.error ? "Try again" : "Connect" },
    ...(f.provider === "gmail" ? [{ keys: "⌥A", label: "Advanced" }] : []),
    esc,
  ];
}

/** Puts the clipboard into the field that takes a pasted password, key or client ID. */
function pasteIntoField() {
  const target = document.querySelector<HTMLInputElement>("[data-paste-target]");
  target?.focus();
  void navigator.clipboard?.readText().then((text) => {
    const f = useAddAccount.getState();
    if (f.provider === "resend") void f.checkResendKey(text);
    else f.setField(f.advanced ? "clientId" : "password", text);
  });
}

function onKey(k: KeyInfo): boolean {
  const f = useAddAccount.getState();
  if (k.key === "escape") {
    if (f.stage === "ai") void f.chooseAi("skip");
    else if (f.stage === "sync") f.finish();
    else f.back();
    return true;
  }
  if (f.stage === "ai") {
    if (k.mod || k.typing) return false;
    if (k.key === "arrowdown") f.moveAiChoice(1);
    else if (k.key === "arrowup") f.moveAiChoice(-1);
    else if (k.key === "enter") void f.chooseAi();
    else if (k.key === "s") void f.chooseAi("skip");
    else return false;
    return true;
  }
  if (f.stage === "provider") {
    if (k.mod || k.typing) return false;
    if (k.digit && k.digit <= 7) f.pick(k.digit - 1);
    else if (k.key === "arrowdown") f.moveCursor(1);
    else if (k.key === "arrowup") f.moveCursor(-1);
    else if (k.key === "enter") f.pick();
    else return false;
    return true;
  }
  if (f.stage === "sync") {
    if (k.mod || k.typing) return false;
    if (k.key === "enter") f.finish();
    else if (k.key === "arrowleft") f.cycleColor(-1);
    else if (k.key === "arrowright") f.cycleColor(1);
    else if (k.key === "a") f.addAnother();
    else return false;
    return true;
  }
  return signInKey(k, f);
}

function signInKey(k: KeyInfo, f: Flow): boolean {
  const steps = stepsOnScreen(f);
  if (k.key === "enter" && !k.mod) {
    if (document.activeElement?.tagName === "BUTTON") return false;
    void f.connect();
    return true;
  }
  if (k.alt && k.key === "a" && (f.provider === "gmail" || f.provider === "outlook")) {
    f.toggleAdvanced();
    return true;
  }
  if (k.mod && k.key === "v" && !k.typing) {
    pasteIntoField();
    return true;
  }
  if (k.mod && k.digit && steps.length > 1) {
    const i = k.digit - 1;
    if (!steps[i]?.url) return false;
    f.openStep(i, steps[i].url!);
    return true;
  }
  if (k.mod && k.key === "o") {
    const page = f.provider === "outlook" && !f.advanced ? accountSetup().signInLink() : steps.find((s) => s.url)?.url;
    if (!page) return false;
    openInBrowser(page);
    return true;
  }
  if (k.mod && k.key === "c" && f.provider === "outlook" && !f.advanced && !window.getSelection()?.toString()) {
    void copyText(f.error ? accountSetup().adminApprovalLink() : accountSetup().signInLink());
    return true;
  }
  if (k.mod && k.key === "e" && f.provider === "imap") {
    document.querySelector<HTMLInputElement>('[aria-label="Incoming · IMAP server"]')?.focus();
    return true;
  }
  if (f.provider === "resend" && !k.mod && !k.typing) return resendKey(k, f);
  return false;
}

function resendKey(k: KeyInfo, f: Flow): boolean {
  if (k.key === "arrowdown") {
    if (k.alt) {
      document.querySelector<HTMLSelectElement>(`[aria-label="Addresses for ${f.resend.domains[f.resend.cursor]?.name}"]`)?.focus();
    } else f.moveDomainCursor(1);
  } else if (k.key === "arrowup") f.moveDomainCursor(-1);
  else if (k.key === " ") f.toggleDomain(f.resend.cursor);
  else return false;
  return true;
}

function SignIn({ f }: { f: Flow }) {
  if (f.advanced) return <OwnClientSignIn provider={f.provider as "gmail" | "outlook"} />;
  if (f.provider === "outlook") return <OutlookSignIn />;
  if (f.provider === "resend") return <ResendSignIn />;
  if (f.provider === "imap") return <ImapSignIn />;
  return <AppPasswordSignIn provider={f.provider as keyof typeof appPasswordCopy} />;
}

export function AddAccountScreen() {
  const f = useAddAccount();
  const ai = useAiSettings((s) => s.status);
  useScreenKeys(onKey);
  const stage = ["provider", "sign-in", "sync", "ai"].indexOf(f.stage);
  const escLabel = ["Cancel", "Back", "Close", "Skip"][stage];
  // The first time, AI on this Mac is a step of its own.
  const steps = f.stage === "ai" || ai?.plan === null ? ["Provider", "Sign in", "Sync", "AI"] : ["Provider", "Sign in", "Sync"];

  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>Add account</PageTitle>}
          right={
            <>
              <FlowSteps steps={steps} current={stage} />
              <span className={styles.gap} />
              <ShortcutHint
                keys="Esc"
                label={escLabel}
                tone="ink-2"
                onClick={() => (f.stage === "ai" ? void f.chooseAi("skip") : f.stage === "sync" ? f.finish() : f.back())}
              />
            </>
          }
        />
      }
      footer={<ShortcutBar hints={hints(f)} />}
    >
      {f.stage === "provider" && <ChooseProvider />}
      {f.stage === "sign-in" && <SignIn f={f} />}
      {f.stage === "sync" && f.connected && <Syncing account={f.connected} />}
      {f.stage === "ai" && f.connected && ai && <AiSetup account={f.connected} status={ai} />}
    </AppFrame>
  );
}
