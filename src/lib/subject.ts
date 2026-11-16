/** A subject without its leading "Re:", "Fw:" and "Fwd:" (any case, any number), like Rust's `strip_re`. */
export function stripRe(subject: string) {
  return subject.trim().replace(/^(?:(?:re|fwd?):\s*)+/i, "");
}

/** The subject of a reply: one "Re:", never "Re: Re:". */
export function replySubject(subject: string) {
  return `Re: ${stripRe(subject)}`;
}
