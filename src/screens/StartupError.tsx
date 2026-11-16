import { useEffect } from "react";
import { TriangleAlert } from "lucide-react";
import { EmptyState } from "../design-system/molecules";
import { watchQuit } from "../services/quit";
import { tryAgain, useStartup } from "../state/startup";
import { QuitDialog } from "./QuitDialog";
import styles from "./StartupError.module.css";

/** In place of the mail when it couldn't be loaded at launch: why, and a way to try again. */
export function StartupError() {
  const failed = useStartup((s) => s.failed);
  const trying = useStartup((s) => s.trying);

  const again = () => {
    if (!useStartup.getState().trying) void tryAgain();
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") again();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ⌘Q asks the window before quitting, so something has to answer it here too.
  useEffect(() => watchQuit(), []);

  return (
    <div className={styles.screen} data-tauri-drag-region>
      <EmptyState
        icon={TriangleAlert}
        title="Moss couldn’t open your mail"
        description={failed!}
        steps={[{ keys: "↵", label: trying ? "Trying again…" : "Try again", onClick: again }]}
      />
      <QuitDialog />
    </div>
  );
}
