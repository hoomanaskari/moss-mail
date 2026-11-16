import { useEffect } from "react";
import { HardDrive, Keyboard, Leaf, Palette, Signature, Users } from "lucide-react";
import { PageTitle } from "../design-system/atoms";
import { ShortcutHint } from "../design-system/molecules";
import { SettingsNav, ShortcutBar, TopBar, type ShortcutBarHint as Hint, type SettingsNavItem } from "../design-system/organisms";
import { AppFrame, SettingsLayout } from "../design-system/templates";
import { useStore, type SettingsSection } from "../state/store";
import { useScreenKeys } from "../state/useScreenKeys";
import { useUpdates } from "../state/updates";
import { AccountsSection } from "./settings/AccountsSection";
import { AiSection } from "./settings/AiSection";
import { AppearanceSection } from "./settings/AppearanceSection";
import { KeyboardSection } from "./settings/KeyboardSection";
import { SignaturesSection } from "./settings/SignaturesSection";
import { StorageSection } from "./settings/StorageSection";

const sections: SettingsNavItem<SettingsSection>[] = [
  { id: "accounts", label: "Accounts", icon: Users, keys: "⌘1" },
  { id: "signatures", label: "Signatures", icon: Signature, keys: "⌘2" },
  { id: "appearance", label: "Appearance", icon: Palette, keys: "⌘3" },
  { id: "ai", label: "AI", icon: Leaf, keys: "⌘4" },
  { id: "storage", label: "Storage", icon: HardDrive, keys: "⌘5" },
  { id: "keyboard", label: "Keyboard shortcuts", icon: Keyboard, keys: "⌘6" },
];

const sectionKeys: Hint = { keys: "⌘1 – ⌘6", label: "Sections" };
const close: Hint = { keys: "Esc", label: "Close" };

/** The keys each section shows along the bottom. */
const hints: Record<SettingsSection, Hint[]> = {
  accounts: [
    { keys: "⌥↑ ⌥↓", label: "Reorder" },
    { keys: "⌘N", label: "Add account" },
    { keys: "⌘⌫", label: "Remove" },
    close,
  ],
  signatures: [
    { keys: "↵", label: "Edit" },
    { keys: "⌘N", label: "New signature" },
    sectionKeys,
    close,
  ],
  appearance: [sectionKeys, { keys: "Tab", label: "Next setting" }, { keys: "← →", label: "Change" }, close],
  ai: [
    { keys: "Space", label: "Choose" },
    { keys: "D", label: "Download model" },
    { keys: "⌘⌫", label: "Delete model" },
    { keys: "A", label: "Your agents" },
    sectionKeys,
    close,
  ],
  storage: [
    { keys: "Space", label: "Choose" },
    { keys: "C", label: "Clear cache" },
    sectionKeys,
    close,
  ],
  keyboard: [{ keys: "?", label: "Show shortcuts anywhere" }, sectionKeys, close],
};

const views: Record<SettingsSection, () => React.JSX.Element> = {
  accounts: AccountsSection,
  signatures: SignaturesSection,
  appearance: AppearanceSection,
  ai: AiSection,
  storage: StorageSection,
  keyboard: KeyboardSection,
};

/** Settings: accounts, signatures, appearance, AI, storage and the list of keys. ⌘1–⌘6 jump between them; Esc goes back. */
export function SettingsScreen() {
  const section = useStore((s) => s.settingsSection);
  const { leave, openSettings } = useStore.getState();
  const View = views[section];
  const version = useUpdates((u) => u.version);

  useEffect(() => {
    if (!version) void useUpdates.getState().load();
  }, [version]);

  useScreenKeys((k) => {
    if (k.key === "escape") return leave(), true;
    const section = k.mod && k.digit ? sections[k.digit - 1] : undefined;
    if (section) return openSettings(section.id), true;
  });

  return (
    <AppFrame
      topBar={
        <TopBar
          title={<PageTitle>Settings</PageTitle>}
          right={<ShortcutHint keys="Esc" label="Close" tone="ink-2" onClick={leave} />}
        />
      }
      footer={<ShortcutBar hints={hints[section]} />}
    >
      <SettingsLayout
        nav={<SettingsNav items={sections} selected={section} onSelect={openSettings} footer={`Moss${version ? ` ${version}` : ""} · Free and open source`} />}
      >
        <View key={section} />
      </SettingsLayout>
    </AppFrame>
  );
}
