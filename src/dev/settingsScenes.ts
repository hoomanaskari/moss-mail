import { demoMail, setMail } from "../services/mail";
import type { BackupStatus } from "../services/backups";
import { showCredentialCleanup } from "../services/credentialCleanup";
import { useRemoteImages } from "../state/remoteImages";
import { gmailDemoAccount } from "../data/demo";
import { demoAi, setAi, type AiFeatures, type DemoAiStart } from "../services/ai";
import { useSignatures } from "../state/signatures";
import { useStore, type SettingsSection } from "../state/store";
import type { Scene } from "./scenes";

/** Settings at a section, with the signed-out Gmail account the design shows as the fourth account. */
const at = (section: SettingsSection) => () => {
  const s = useStore.getState();
  s.addAccount(gmailDemoAccount);
  s.openSettings(section);
};

const all: AiFeatures = { search: true, sorting: true, writing: true };

/** Settings → AI with the demo AI at a state drawn in design.pen. */
const aiWith = (start: DemoAiStart) => () => {
  setAi(demoAi(start));
  at("ai")();
};

const backupWith = (status: BackupStatus) => () => {
  setMail({ ...demoMail, backupStatus: async () => status });
  at("storage")();
};

export const settingsScenes: Record<string, Scene> = {
  "settings-backup-running": { shows: /Copying local mail/, apply: backupWith({state:"running",id:7,cancelling:false,progress:{phase:"copying",copiedPages:128,totalPages:500}}) },
  "settings-backup-cancelling": { shows: /Cancelling/, apply: backupWith({state:"running",id:7,cancelling:true,progress:{phase:"checking"}}) },
  "settings-backup-complete": { shows: /Archive saved/, apply: backupWith({state:"complete",id:7,bytes:1048576,sha256:"a".repeat(64)}) },
  "settings-backup-failed": { shows: /The backup could not be written/, apply: backupWith({state:"failed",id:7,message:"The backup could not be written."}) },
  "settings-credential-cleanup": {
    shows: /Credential cleanup is pending/,
    apply: () => { at("accounts")(); showCredentialCleanup({ pending: true }); },
  },
  "settings-images-strict": {
    shows: /Loading remote images can reveal your IP address/,
    apply: () => {
      useRemoteImages.setState({ policy: "ask", saved: true });
      at("appearance")();
    },
  },
  "settings-accounts": { shows: /Editing Work/, apply: at("accounts") },
  "settings-signatures": { shows: /Let Moss choose/, apply: at("signatures") },
  "settings-signatures-empty": {
    shows: /No signatures yet/,
    apply: () => {
      useSignatures.setState({ signatures: [], byAccount: {} });
      at("signatures")();
    },
  },
  "settings-appearance": {
    shows: /Reading size/i,
    apply: () => {
      // The design shows "System" chosen while drawn in the URL's theme: make the system agree.
      const light = useStore.getState().theme === "light";
      window.matchMedia = ((query: string) => ({
        matches: query.includes("light") === light,
        addEventListener: () => {},
        removeEventListener: () => {},
      })) as unknown as typeof window.matchMedia;
      useStore.getState().setTheme("system");
      at("appearance")();
    },
  },
  "settings-ai": { shows: /Sorting and search download on their own/, apply: at("ai") },
  "settings-ai-downloading": {
    shows: /Downloading… 2\.1 of 3\.1 GB · 67%/,
    apply: aiWith({ features: all, models: { e2b: { state: "downloading", done: 2_100_000_000 } } }),
  },
  "settings-ai-paused": {
    shows: /Paused at 1\.2 of 3\.1 GB/,
    apply: aiWith({ features: { search: true, sorting: true, writing: false }, models: { e2b: { state: "missing", done: 1_200_000_000 } } }),
  },
  "settings-ai-failed": {
    shows: /Can’t reach the download server/,
    apply: aiWith({
      features: { search: true, sorting: true, writing: false },
      models: { e4b: { state: "failed", done: 2_400_000_000, message: "Can’t reach the download server. Check your connection and try again." } },
    }),
  },
  "settings-ai-ready": { shows: /^Ready$/, apply: aiWith({ features: all }) },
  "settings-ai-apple": { shows: /Built into macOS: quick, with nothing to download\./, apply: aiWith({ apple: true, features: { search: true, sorting: true, writing: true } }) },
  "settings-ai-took-over": { shows: /Gemma 4 E2B is ready and now writes for you\./, apply: aiWith({ features: all, have: ["e2b"], writer: "e2b", tookOver: "e2b" }) },
  "settings-ai-finished-not-chosen": { shows: /Fastest\. Good for summaries and quick replies\./, apply: aiWith({ features: all, have: ["e2b"] }) },
  "settings-storage": { shows: /142 MB of 500 MB/, apply: at("storage") },
  "settings-keyboard": { shows: /Remind me if no reply/, apply: at("keyboard") },
};
