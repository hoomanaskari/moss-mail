/** Direction controls must not disguise addresses or filenames in send-review labels. */
export const plainLabel = (text: string) => text.replace(/[\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/g, "");
