import { describe, expect, it } from "vitest";
import { senderImages } from "./senderImages";

describe("senderImages", () => {
  it("shows a trusted sender's images only when the mail server confirmed they sent it", () => {
    expect(senderImages({ imageTrust: "you-wrote" })).toEqual({ imagesAllowed: true, senderUnconfirmed: false });
    expect(senderImages({ imageTrust: "yours", senderUnverified: true })).toEqual({ imagesAllowed: false, senderUnconfirmed: true });
    expect(senderImages({ imageTrust: "ask", senderUnverified: true })).toEqual({ imagesAllowed: false, senderUnconfirmed: false });
    expect(senderImages({})).toEqual({ imagesAllowed: false, senderUnconfirmed: false });
  });
});
