import Foundation
import FoundationModels

/// Tool and answer shapes, from the JSON schemas Moss writes to ones Apple's model reads.
///
/// Apple reads a subset of JSON schema, and needs more on each object than OpenAI does: a
/// `title`, the order to write the properties in (`x-order`), `required` even when empty, and no
/// other properties. A value that may be null is just its type, left out of `required`.
public enum Schema {
    /// The keys Apple reads; others (`format`, `minimum`, `default`…) are left out.
    static let kept: Set<String> = ["type", "description", "properties", "required", "items", "enum"]

    static func normalized(_ value: Any, title: String) -> Any {
        guard var object = value as? [String: Any] else { return value }
        if let choices = object["anyOf"] as? [Any] ?? object["oneOf"] as? [Any] {
            let other = choices.compactMap { $0 as? [String: Any] }.first { ($0["type"] as? String) != "null" } ?? [:]
            object = object.merging(other) { mine, _ in mine }
        }
        object = object.filter { kept.contains($0.key) }
        if let types = object["type"] as? [Any] {
            object["type"] = types.compactMap { $0 as? String }.first { $0 != "null" } ?? "string"
        }
        if let properties = object["properties"] as? [String: Any] {
            object["properties"] = properties.reduce(into: [String: Any]()) { out, p in
                out[p.key] = normalized(p.value, title: "\(title)_\(p.key)")
            }
            let required = (object["required"] as? [String] ?? []).filter { properties[$0] != nil }
            object["required"] = required
            object["x-order"] = required + properties.keys.sorted().filter { !required.contains($0) }
            object["title"] = title
            object["additionalProperties"] = false
        }
        if let items = object["items"] {
            object["items"] = normalized(items, title: "\(title)_item")
        }
        return object
    }

    /// The shape as Apple's model takes it. Throws when Apple can't read it.
    static func generation(_ value: Any, title: String) throws -> GenerationSchema {
        let data = try JSONSerialization.data(withJSONObject: normalized(value, title: title))
        return try JSONDecoder().decode(GenerationSchema.self, from: data)
    }
}
