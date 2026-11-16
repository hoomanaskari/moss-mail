import { render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { ImageTrust } from "../../../data/types";
import { useRemoteImages } from "../../../state/remoteImages";
import { message, work } from "../../../test/fixtures";
import { ForwardedMessage } from "../ForwardedMessage/ForwardedMessage";
import { MessageView } from "../MessageView/MessageView";

afterEach(() => useRemoteImages.setState({ policy: "familiar", saved: true }));
const html = '<img src="https://images.example.test/visible.png" width="300" height="200"><style>p { background-image:url(https://images.example.test/background.png) }</style><p>Mail</p>';
const noop = () => {};
for (const surface of ["message", "forward"] as const) {
  it.each<ImageTrust>(["ask", "yours", "you-allowed", "you-wrote", "same-company"])(`${surface}: strict mode requires explicit permission for %s`, (imageTrust) => {
    useRemoteImages.setState({ policy: "ask" });
    const mail = message({ html, imageTrust });
    const view = (shown: boolean) => surface === "forward"
      ? <ForwardedMessage message={mail} subject="Preview" imagesShown={shown} onShowImages={noop} onAllowImages={noop} />
      : <MessageView message={mail} account={work} recipientsOpen={false} onToggleRecipients={noop} imagesShown={shown} />;
    const { rerender } = render(view(false));
    const frame = () => (screen.getByTitle("Email") as HTMLIFrameElement).srcdoc;
    expect(frame()).not.toContain("images.example.test");
    expect(screen.queryByRole("button", { name: /Always for this sender/ })).not.toBeInTheDocument();
    rerender(view(true));
    expect(frame()).toContain('src="https://images.example.test/visible.png"');
    expect(frame()).toContain("https://images.example.test/background.png");
  });
}
