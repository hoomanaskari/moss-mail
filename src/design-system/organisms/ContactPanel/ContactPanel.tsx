import { MapPin } from "lucide-react";
import type { Between, Person } from "../../../data/types";
import { Avatar, Icon, SectionLabel } from "../../atoms";
import styles from "./ContactPanel.module.css";
import { SEP } from "../../../lib/text";

export interface ContactPanelProps {
  person: Person;
  recent?: { subject: string; when: string }[];
  /** Something the AI noticed about how you work with this person. */
  insight?: string;
  between?: Between;
}

/** About the sender, in the card their name opens: who they are, their local time, what you share, recent emails. */
export function ContactPanel({ person, recent = [], insight, between }: ContactPanelProps) {
  return (
    <div className={styles.panel}>
      <div className={styles.who}>
        <Avatar initials={person.initials} size={56} />
        <span className={styles.gap} />
        <span className={styles.name}>{person.name}</span>
        {person.role && <span className={styles.role}>{person.role}</span>}
        {person.email !== person.name && <span className={styles.email}>{person.email}</span>}
      </div>
      {person.location && (
        <div className={styles.time}>
          <Icon icon={MapPin} size={15} tone="ink-3" />
          <span>
            {person.location}
            {person.localTime && `${SEP}${person.localTime} there`}
          </span>
        </div>
      )}
      {between && (
        <div className={styles.section}>
          <SectionLabel size="sm" as="h3">
            Between you
          </SectionLabel>
          {(
            [
              ["Emails together", between.emails],
              ["First email", between.since],
              between.youWrote !== undefined
                ? ["You’ve written to them", between.youWrote]
                : ["You usually reply", between.youReply],
            ] as const
          ).map(([label, value]) => (
            <div key={label} className={styles.stat}>
              <span className={styles.statLabel}>{label}</span>
              <span className={styles.statValue}>{value}</span>
            </div>
          ))}
        </div>
      )}
      {recent.length > 0 && (
        <div className={styles.section}>
          <SectionLabel size="sm" as="h3">
            Recent
          </SectionLabel>
          {recent.map((r) => (
            <div key={r.subject} className={styles.recentItem}>
              <span className={styles.recentSubject}>{r.subject}</span>
              <span className={styles.recentWhen}>{r.when}</span>
            </div>
          ))}
        </div>
      )}
      {insight && (
        <div className={styles.insight}>
          <span className={styles.insightLabel}>Good to know</span>
          <p className={styles.insightText}>{insight}</p>
        </div>
      )}
    </div>
  );
}
