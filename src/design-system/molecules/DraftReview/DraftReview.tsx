import { Fragment } from "react";
import { plainLabel } from "../../../lib/displayLabel";
import type { Draft } from "../../../state/store";
import { fileSize } from "../../../lib/attachments";
import styles from "./DraftReview.module.css";

export type DraftReviewDetails = Pick<Draft, "to" | "cc" | "bcc" | "attachments"> & { from: string };

/** Full addresses and files remain visible wherever a draft can be sent. */

export function DraftReview({ from, to, cc, bcc, attachments = [] }: DraftReviewDetails) {
  return (
    <div className={styles.review}>
      <dl className={styles.recipients}>
        <div><dt>From</dt><dd><bdi dir="ltr">{plainLabel(from)}</bdi></dd></div>
        {([["To", to], ["Cc", cc], ["Bcc", bcc]] as const).filter(([, people]) => people.length > 0).map(([label, people]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{people.map((p, index) => (
              <Fragment key={`${p.email}-${index}`}>
                {index > 0 && ", "}
                <bdi dir="ltr">{p.name && p.name !== p.email ? `${plainLabel(p.name)} <${plainLabel(p.email)}>` : plainLabel(p.email)}</bdi>
              </Fragment>
            ))}</dd>
          </div>
        ))}
      </dl>
      {attachments.length > 0 && (
        <ul className={styles.files} aria-label="Attachments">
          {attachments.map((file) => <li key={file.id}><span><bdi dir="ltr">{plainLabel(file.name)}</bdi></span><span>{fileSize(file.bytes)}</span></li>)}
        </ul>
      )}
    </div>
  );
}
