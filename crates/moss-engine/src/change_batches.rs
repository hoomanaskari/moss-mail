//! Byte-bounded encoding of one logical server action. Persistence owns atomicity.
use super::ServerChange;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EncodeError { InvalidTarget, TooLarge, TooManyParts }

pub(crate) fn encode(changes: Vec<ServerChange>) -> moss_store::Result<Vec<String>> {
    encode_with_limits(changes, moss_store::MAX_CHANGE_PAYLOAD_BYTES, moss_store::MAX_CHANGE_PARTS)
        .map_err(|_| moss_store::Error::Invalid("This action contains server targets that cannot be queued safely. No local changes were saved.".into()))
}

fn encode_with_limits(changes: Vec<ServerChange>, budget: usize, max_parts: usize) -> Result<Vec<String>, EncodeError> {
    let mut parts = Vec::new();
    for mut change in changes {
        let (targets, uids) = match &mut change {
            ServerChange::Archive { ids, .. } | ServerChange::Seen { ids, .. }
            | ServerChange::Flagged { ids, .. } | ServerChange::Forwarded { ids, .. } => (std::mem::take(ids), true),
            ServerChange::Unarchive { message_ids } | ServerChange::Trash { message_ids }
            | ServerChange::Untrash { message_ids, .. } | ServerChange::DeleteDrafts { message_ids } => (std::mem::take(message_ids), false),
        };
        if targets.is_empty() { continue; }
        let envelope = serde_json::to_string(&change).expect("server changes serialise");
        // Match the complete array field, not an arbitrary [] inside a mailbox name.
        let field = if uids { "\"ids\":[]" } else { "\"messageIds\":[]" };
        let (before, after) = envelope.split_once(field).expect("server change has its target field");
        let prefix = format!("{}{}", before, &field[..field.len()-1]);
        let suffix = format!("]{after}");
        let capacity = budget.checked_sub(envelope.len()).ok_or(EncodeError::TooLarge)?;
        let mut part = prefix.clone();
        let mut used = 0;
        let mut count = 0;
        for target in targets {
            // Reject the raw lower bound before scanning or allocating escaped text.
            if target.len() > capacity { return Err(EncodeError::TooLarge); }
            let valid = if uids {
                target.split_once(':').and_then(|(v,u)| Some((v.parse::<u32>().ok()?, u.parse::<u32>().ok()?)))
                    .is_some_and(|(v,u)| v > 0 && u > 0)
            } else {
                moss_providers::searchable_message_id(&target)
            };
            if !valid { return Err(EncodeError::InvalidTarget); }
            let encoded = serde_json::to_string(&target).expect("strings serialise");
            if encoded.len() > capacity { return Err(EncodeError::TooLarge); }
            let separator = usize::from(count > 0);
            if encoded.len() + separator > capacity - used {
                part.push_str(&suffix);
                if parts.len() == max_parts { return Err(EncodeError::TooManyParts); }
                parts.push(part);
                part = prefix.clone();
                used = 0;
                count = 0;
            }
            if count > 0 { part.push(','); used += 1; }
            part.push_str(&encoded);
            used += encoded.len();
            count += 1;
        }
        part.push_str(&suffix);
        if parts.len() == max_parts { return Err(EncodeError::TooManyParts); }
        parts.push(part);
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variants() -> Vec<ServerChange> {
        let ids = vec!["42:1".into(), "42:2".into(), "42:3".into()];
        let message_ids = vec!["first@example.test".into(), "second@example.test".into(), "third@example.test".into()];
        vec![
            ServerChange::Archive { mailbox: "箱\"\n".into(), ids: ids.clone() },
            ServerChange::Seen { mailbox: "Inbox".into(), ids: ids.clone(), seen: false },
            ServerChange::Flagged { mailbox: "Inbox".into(), ids: ids.clone(), flagged: true },
            ServerChange::Forwarded { mailbox: "Inbox".into(), ids },
            ServerChange::Unarchive { message_ids: message_ids.clone() },
            ServerChange::Trash { message_ids: message_ids.clone() },
            ServerChange::Untrash { message_ids: message_ids.clone(), to_inbox: true },
            ServerChange::Untrash { message_ids: message_ids.clone(), to_inbox: false },
            ServerChange::DeleteDrafts { message_ids },
        ]
    }

    #[test]
    fn every_variant_round_trips_in_order_with_exact_byte_boundaries() {
        for change in variants() {
            let original = serde_json::to_value(&change).unwrap();
            let key = if original.get("ids").is_some() { "ids" } else { "messageIds" };
            let targets = original[key].as_array().unwrap();
            let mut singleton = original.clone();
            singleton[key] = serde_json::json!([targets[0]]);
            let budget = serde_json::to_string(&singleton).unwrap().len() + 1;
            let parts = encode_with_limits(vec![change], budget, 10).unwrap();
            assert!(parts.len() > 1);
            let mut joined = Vec::new();
            for part in parts {
                assert!(part.len() <= budget);
                let mut decoded: serde_json::Value = serde_json::from_str(&part).unwrap();
                joined.extend(decoded[key].as_array().unwrap().iter().cloned());
                decoded[key] = original[key].clone();
                assert_eq!(decoded, original);
            }
            assert_eq!(&joined, targets);
        }
        let change = ServerChange::Trash { message_ids: vec!["x@y".into()] };
        let exact = serde_json::to_string(&change).unwrap().len();
        assert_eq!(encode_with_limits(vec![change.clone()], exact, 1).unwrap(), vec![serde_json::to_string(&change).unwrap()]);
        assert_eq!(encode_with_limits(vec![change], exact - 1, 1), Err(EncodeError::TooLarge));
    }

    #[test]
    fn production_limit_splits_large_actions_and_preserves_action_order() {
        let targets: Vec<String> = (0..4096).map(|n| format!("{n}-{}@example.test", "x".repeat(300))).collect();
        let original = ServerChange::Trash { message_ids: targets.clone() };
        let mut changes = vec![original];
        changes.extend(variants());
        let parts = encode(changes).unwrap();
        assert!(parts.len() > 10);
        let mut joined = Vec::new();
        let trash_parts = parts.len() - variants().len();
        for part in &parts[..trash_parts] {
            assert!(part.len() <= moss_store::MAX_CHANGE_PAYLOAD_BYTES);
            let decoded: serde_json::Value = serde_json::from_str(part).unwrap();
            assert_eq!(decoded["kind"], "trash");
            joined.extend(serde_json::from_value::<Vec<String>>(decoded["messageIds"].clone()).unwrap());
        }
        assert_eq!(joined, targets);
        let rest: Vec<ServerChange> = parts[trash_parts..].iter().map(|part| serde_json::from_str(part).unwrap()).collect();
        assert_eq!(rest, variants());
        let error = encode(vec![ServerChange::Trash { message_ids: vec!["private invalid target".into()] }]).unwrap_err();
        assert_eq!(error.to_string(), "This action contains server targets that cannot be queued safely. No local changes were saved.");
    }

    #[test]
    fn oversized_raw_targets_are_refused_before_content_validation() {
        for change in [
            ServerChange::Trash { message_ids: vec![" ".repeat(1024)] },
            ServerChange::Seen { mailbox: "Inbox".into(), ids: vec!["not-a-uid".repeat(128)], seen: true },
        ] {
            assert_eq!(encode_with_limits(vec![change],100,1),Err(EncodeError::TooLarge));
        }
    }

    #[test]
    fn invalid_targets_and_part_overflow_reject_the_whole_action() {
        for id in ["", "a b", "a\"b", "a\\b", "a\nb", "é@x"] {
            assert_eq!(encode_with_limits(vec![ServerChange::Trash { message_ids: vec!["good@x".into(), id.into()] }], 200, 10), Err(EncodeError::InvalidTarget));
        }
        for id in ["", "0:1", "1:0", "1", "1:2:3", "4294967296:1"] {
            assert_eq!(encode_with_limits(vec![ServerChange::Seen { mailbox: "Inbox".into(), ids: vec![id.into()], seen: true }], 200, 10), Err(EncodeError::InvalidTarget));
        }
        assert_eq!(encode_with_limits(variants(), 200, 1), Err(EncodeError::TooManyParts));
        assert_eq!(encode_with_limits(vec![ServerChange::Trash { message_ids: vec!["x@y".into(); 3] }], 40, 1), Err(EncodeError::TooManyParts));
        assert_eq!(encode_with_limits(vec![ServerChange::Trash { message_ids: vec!["x@y".into()] }], 0, 1), Err(EncodeError::TooLarge));
        assert!(encode_with_limits(vec![], 0, 0).unwrap().is_empty());
        assert!(encode_with_limits(vec![ServerChange::Trash { message_ids: vec![] }], 0, 0).unwrap().is_empty());
    }
}
