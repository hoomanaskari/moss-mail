/** A subject without its leading "Re:", "Fw:" and "Fwd:" (any case, any number), like Rust's `strip_re`. */
export function stripRe(subject: string) {
  return subject.trim().replace(/^(?:(?:re|fwd?):\s*)+/i, "");
}

/** The subject of a reply: one "Re:", never "Re: Re:". */
export function replySubject(subject: string) {
  return `Re: ${stripRe(subject)}`;
}

/**
 * The subject of an answer: it starts with "Re:". A forward ("Fwd: Re: …") doesn't.
 * Other languages' prefixes ("AW:") read as not a reply, which only ever shows more of an email, never less.
 */
export function isReplySubject(subject: string) {
  return /^\s*re\s*:/i.test(subject);
}
