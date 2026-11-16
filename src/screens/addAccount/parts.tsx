import { useState, type ComponentProps } from "react";
import { CircleAlert, Eye, EyeOff, Lock } from "lucide-react";
import { Button, Icon, Kbd, TextInput } from "../../design-system/atoms";
import { Notice } from "../../design-system/molecules";
import { useAddAccount } from "../../state/addAccount";
import styles from "./AddAccount.module.css";

/** Why connecting didn't work, when it wasn't the password: no connection, no settings found… */
export function SetupProblem() {
  const { error, errorMessage } = useAddAccount();
  if (error !== "failed") return null;
  return (
    <Notice variant="caution" icon={CircleAlert}>
      {errorMessage}
    </Notice>
  );
}

/** The grey Keychain line that ends every sign-in step. */
export function Footnote({ children }: { children: string }) {
  return (
    <Notice variant="footnote" icon={Lock}>
      {children}
    </Notice>
  );
}

/** "Advanced: Use your own … ⌥A" */
export function AdvancedLink({ label, onClick, prefix = true }: { label: string; onClick: () => void; prefix?: boolean }) {
  return (
    <span className={styles.advanced}>
      {prefix && <span className={styles.advancedLabel}>Advanced:</span>}
      <Button variant="link" shortcut="⌥A" onClick={onClick}>
        {label}
      </Button>
    </span>
  );
}

type SecretProps = Omit<ComponentProps<typeof TextInput>, "type" | "trailing"> & {
  /** Show an eye button that reveals the text. */
  reveal?: boolean;
  /** Show the ⌘V paste key. */
  paste?: boolean;
};

/** A password or key field, with an optional eye to show it and the ⌘V paste key. */
export function SecretInput({ reveal, paste, ...rest }: SecretProps) {
  const [shown, setShown] = useState(false);
  return (
    <TextInput
      type={shown ? "text" : "password"}
      autoComplete="off"
      trailing={
        <>
          {reveal && (
            <button type="button" className={styles.eye} aria-label={shown ? "Hide" : "Show"} onClick={() => setShown(!shown)}>
              <Icon icon={shown ? EyeOff : Eye} size={16} tone="inherit" />
            </button>
          )}
          {paste && <Kbd>⌘V</Kbd>}
        </>
      }
      {...rest}
    />
  );
}
