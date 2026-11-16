import Foundation
import FoundationModels
import Testing

@testable import MossAppleCore

private func json(_ text: String) -> Any {
    try! JSONSerialization.jsonObject(with: Data(text.utf8))
}

private func object(_ value: Any) -> [String: Any] {
    value as! [String: Any]
}

@Suite struct SchemaTests {
    @Test func anObjectGetsTheKeysAppleNeeds() {
        let out = object(Schema.normalized(json(#"{"type":"object","properties":{"b":{"type":"string"},"a":{"type":"string"},"c":{"type":"string"}},"required":["c"]}"#), title: "pick"))
        #expect(out["title"] as? String == "pick")
        #expect(out["additionalProperties"] as? Bool == false)
        // What it has to say first, then the rest in the order the names sort.
        #expect(out["x-order"] as? [String] == ["c", "a", "b"])
        #expect(out["required"] as? [String] == ["c"])
    }

    @Test func anObjectWithNothingRequiredSaysSo() {
        let out = object(Schema.normalized(json(#"{"type":"object","properties":{}}"#), title: "list_drafts"))
        #expect(out["required"] as? [String] == [])
        #expect(out["x-order"] as? [String] == [])
    }

    @Test func aTypeThatMayBeNullIsJustThatType() {
        let out = object(Schema.normalized(json(#"{"type":"object","properties":{"done":{"type":["boolean","null"],"description":"d"},"n":{"type":["null","integer"],"format":"uint","minimum":0}}}"#), title: "t"))
        let properties = object(out["properties"]!)
        #expect(object(properties["done"]!)["type"] as? String == "boolean")
        #expect(object(properties["done"]!)["description"] as? String == "d")
        let n = object(properties["n"]!)
        #expect(n["type"] as? String == "integer")
        #expect(n["format"] == nil)
        #expect(n["minimum"] == nil)
    }

    @Test func aTypeOfOnlyNullBecomesText() {
        let out = object(Schema.normalized(json(#"{"type":["null"]}"#), title: "t"))
        #expect(out["type"] as? String == "string")
    }

    @Test func listsAndNestedObjectsAreNormalizedToo() {
        let out = object(Schema.normalized(json(#"{"type":"object","properties":{"to":{"type":["array","null"],"items":{"type":"object","properties":{"email":{"type":"string"}}}}}}"#), title: "write_email"))
        let to = object(object(out["properties"]!)["to"]!)
        #expect(to["type"] as? String == "array")
        let item = object(to["items"]!)
        #expect(item["title"] as? String == "write_email_to_item")
        #expect(item["x-order"] as? [String] == ["email"])
    }

    @Test func aChoiceThatMayBeNullIsTheOtherChoice() {
        let out = object(Schema.normalized(json(#"{"anyOf":[{"type":"null"},{"type":"string","enum":["a","b"]}],"description":"which"}"#), title: "t"))
        #expect(out["type"] as? String == "string")
        #expect(out["enum"] as? [String] == ["a", "b"])
        #expect(out["description"] as? String == "which")
        #expect(out["anyOf"] == nil)
    }

    @Test func aChoiceOfOnlyNullIsLeftAsItsOtherWords() {
        let out = object(Schema.normalized(json(#"{"anyOf":[{"type":"null"}],"description":"mine","type":"string"}"#), title: "t"))
        #expect(out["type"] as? String == "string")
        let both = object(Schema.normalized(json(#"{"oneOf":[{"type":"string","description":"theirs"}],"description":"mine"}"#), title: "t"))
        // Its own words win over the choice's.
        #expect(both["description"] as? String == "mine")
        #expect(both["type"] as? String == "string")
    }

    @Test func somethingThatIsNotASchemaIsLeftAlone() {
        #expect(Schema.normalized("text", title: "t") as? String == "text")
    }

    @Test func keysAppleDoesNotReadAreLeftOut() {
        let out = object(Schema.normalized(json(#"{"type":"string","default":"x","$schema":"s","pattern":"a+"}"#), title: "t"))
        #expect(out.keys.sorted() == ["type"])
    }

    @Test func aNormalizedSchemaIsOneAppleCanRead() throws {
        let tool = #"{"type":"object","properties":{"thread_id":{"type":"string","description":"The thread's id."},"done":{"type":["boolean","null"]},"labels":{"type":"array","items":{"type":"string"}},"split":{"type":"string","enum":["important","other"]}},"required":["thread_id"]}"#
        let schema = try Schema.generation(json(tool), title: "mark_done")
        #expect(schema.debugDescription.contains("thread_id"))
        let empty = try Schema.generation(json(#"{"type":"object","properties":{}}"#), title: "list_drafts")
        #expect(empty.debugDescription.contains("list_drafts"))
    }

    @Test func aSchemaAppleCannotReadIsAnError() {
        #expect(throws: (any Error).self) { try Schema.generation(json(#"{"type":"object","properties":{"a":{"type":"nonsense"}}}"#), title: "t") }
    }
}
