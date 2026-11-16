import { Component, type ReactNode } from "react";
import { TriangleAlert } from "lucide-react";
import { EmptyState } from "../../molecules";
import styles from "./ScreenErrorBoundary.module.css";

interface Props {
  children: ReactNode;
  /** Takes you somewhere safe, e.g. back to the inbox. */
  onRecover: () => void;
}

interface State {
  failed: boolean;
}

/** If a screen crashes, show a calm way back instead of a blank window. */
export class ScreenErrorBoundary extends Component<Props, State> {
  state: State = { failed: false };

  static getDerivedStateFromError(): State {
    return { failed: true };
  }

  componentDidCatch(error: unknown) {
    console.error("Moss screen error:", error);
  }

  private recover = () => {
    this.setState({ failed: false });
    this.props.onRecover();
  };

  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className={styles.screen}>
        <EmptyState
          icon={TriangleAlert}
          title="Something went wrong on this screen"
          description="Your emails and drafts are safe. Go back to the inbox to carry on."
          steps={[{ keys: "Esc", label: "Back to inbox", onClick: this.recover }]}
        />
      </div>
    );
  }
}
