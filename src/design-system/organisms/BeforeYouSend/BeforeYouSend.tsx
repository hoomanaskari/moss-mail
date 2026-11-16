import { Paperclip, Users } from "lucide-react";
import { Button } from "../../atoms";
import { Notice } from "../../molecules";
import styles from "./BeforeYouSend.module.css";

export interface BeforeYouSendProps {
  /** e.g. "You mention attached notes, but nothing is attached." */
  missingAttachment?: string;
  /** e.g. "Goes to 4 people and 1 BCC. Ines is outside Northwind." */
  audience: string;
  onAttach: () => void;
}

/** Last checks above the Send button: forgotten attachments and who will get the email. */
export function BeforeYouSend({ missingAttachment, audience, onAttach }: BeforeYouSendProps) {
  return (
    <div className={styles.checks}>
      {missingAttachment && (
        <Notice
          variant="warn"
          icon={Paperclip}
          action={
            <Button variant="warn" onClick={onAttach}>
              Attach file
            </Button>
          }
        >
          {missingAttachment}
        </Notice>
      )}
      <Notice variant="plain" icon={Users}>
        {audience}
      </Notice>
    </div>
  );
}
