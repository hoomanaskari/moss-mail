import { SectionHeader, SegmentedControl } from "../../design-system/molecules";
import { useRemoteImages } from "../../state/remoteImages";
import shared from "./settings.module.css";
import styles from "./RemoteImagesSettings.module.css";

export function RemoteImagesSettings() {
  const { policy, saved, setPolicy } = useRemoteImages();
  return <div className={shared.group}>
    <SectionHeader label="Remote images" />
    <SegmentedControl label="Remote images" value={policy} onChange={setPolicy} options={[
      { value: "familiar", label: "Familiar senders" },
      { value: "ask", label: "Ask for every message" },
    ]} />
    <p className={styles.explanation}>Familiar senders includes people you have written to, their company and senders you allowed. Ask for every message hides remote images until you choose Just this once, even for familiar senders.</p>
    <p className={styles.explanation}>Loading remote images can reveal your IP address and when you read an email. Moss removes possible trackers using rules, but cannot detect every tracker or hide your IP address.</p>
    {!saved && <p role="status" className={styles.explanation}>This choice could not be saved. It applies until Moss closes.</p>}
  </div>;
}
