import type { Message } from "../../../data/types";

/**
 * Whether a message's images load by themselves because of who it's from, and whether that
 * trust was held back because the mail server didn't confirm the sender.
 */
export function senderImages(message: Pick<Message, "imageTrust" | "senderUnverified">) {
  const trusted = (message.imageTrust ?? "ask") !== "ask";
  return { imagesAllowed: trusted && !message.senderUnverified, senderUnconfirmed: trusted && !!message.senderUnverified };
}
