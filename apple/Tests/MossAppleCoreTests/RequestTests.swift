import Foundation
import Testing

@testable import MossAppleCore

private func request(_ text: String) throws -> ChatRequest {
    try ChatRequest.parse(Data(text.utf8))
}

private let searchTool = #"{"type":"function","function":{"name":"search_mail","description":"Find emails","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}}"#

@Suite struct RequestTests {
    @Test func systemMessagesAreTheInstructionsAndTheLastUserMessageIsThePrompt() throws {
        let r = try request(#"{"messages":[{"role":"system","content":"Be brief."},{"role":"system","content":"Be kind."},{"role":"user","content":"Hi"}]}"#)
        #expect(r.instructions == "Be brief.\n\nBe kind.")
        #expect(r.history.isEmpty)
        #expect(r.prompt == "Hi")
        #expect(r.tools.isEmpty)
        #expect(r.mustCall == false)
        #expect(r.format == nil)
        #expect(r.maxTokens == nil)
        #expect(r.temperature == nil)
    }

    @Test func earlierTurnsAreTheHistory() throws {
        let r = try request(#"{"messages":[{"role":"system","content":"s"},{"role":"user","content":"Q1"},{"role":"assistant","content":"A1"},{"role":"user","content":"Q2"}],"max_tokens":50,"temperature":0.2}"#)
        #expect(r.history == [.user("Q1"), .assistant("A1")])
        #expect(r.prompt == "Q2")
        #expect(r.maxTokens == 50)
        #expect(r.temperature == 0.2)
    }

    @Test func toolCallsAndTheirResultsAreHistoryAndTheModelGoesOnFromThem() throws {
        let r = try request(#"""
        {"messages":[{"role":"user","content":"Mark Framely done"},
          {"role":"assistant","content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"search_mail","arguments":"{\"query\":\"Framely\"}"}}]},
          {"role":"tool","tool_call_id":"c1","content":"[t:41] Config recap"}],
         "tools":[\#(searchTool)],"tool_choice":"required","parallel_tool_calls":true}
        """#)
        #expect(r.history == [
            .user("Mark Framely done"),
            .calls([Call(id: "c1", name: "search_mail", arguments: #"{"query":"Framely"}"#)]),
            .output(id: "c1", name: "search_mail", text: "[t:41] Config recap"),
        ])
        #expect(r.prompt == "")
        #expect(r.mustCall)
        #expect(r.tools.map(\.name) == ["search_mail"])
        #expect(r.tools[0].description == "Find emails")
    }

    @Test func wordsBeforeToolCallsAreKept() throws {
        let r = try request(#"""
        {"messages":[{"role":"user","content":"q"},
          {"role":"assistant","content":"Looking.","tool_calls":[{"id":"c1","function":{"name":"find","arguments":"{}"}}]},
          {"role":"tool","tool_call_id":"c9","content":"r"}]}
        """#)
        #expect(r.history == [
            .user("q"), .assistant("Looking."), .calls([Call(id: "c1", name: "find", arguments: "{}")]),
            // A result for a call it never made still reaches the model, under no tool's name.
            .output(id: "c9", name: "", text: "r"),
        ])
    }

    @Test func aChatEndingInTheModelsOwnWordsGoesOnFromThem() throws {
        let r = try request(#"{"messages":[{"role":"user","content":"q"},{"role":"assistant","content":"a"}]}"#)
        #expect(r.history == [.user("q"), .assistant("a")])
        #expect(r.prompt == "")
    }

    @Test func contentInPartsIsJoined() throws {
        let r = try request(#"{"messages":[{"role":"user","content":[{"type":"text","text":"one"},{"type":"image_url"},{"type":"text","text":"two"}]}]}"#)
        #expect(r.prompt == "one\ntwo")
    }

    @Test func theAnswersShapeComesFromTheResponseFormat() throws {
        let r = try request(#"{"messages":[{"role":"user","content":"q"}],"response_format":{"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object","properties":{"done":{"type":"boolean"}}}}}}"#)
        let format = try #require(r.format)
        #expect(format.name == "answer")
        #expect((format.schema as? [String: Any])?["type"] as? String == "object")
    }

    @Test func toolChoiceOtherThanRequiredLeavesItToTheModel() throws {
        let r = try request(#"{"messages":[{"role":"user","content":"q"}],"tool_choice":"auto"}"#)
        #expect(r.mustCall == false)
    }

    @Test func missingPartsGetPlainDefaults() throws {
        let r = try request(#"""
        {"messages":[{"role":"user","content":"q"},{"role":"assistant","tool_calls":[{}]},{"role":"tool","content":"r"},{"content":"next"}],
         "tools":[{"type":"function","function":{"name":"list_drafts"}}],
         "response_format":{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}}
        """#)
        #expect(r.history == [.user("q"), .calls([Call(id: "", name: "", arguments: "{}")]), .output(id: "", name: "", text: "r")])
        // A message with no role is the user's.
        #expect(r.prompt == "next")
        #expect(r.tools.first?.description == "")
        #expect((r.tools.first?.parameters as? [String: Any])?["type"] as? String == "object")
        #expect(r.format?.name == "answer")
    }

    @Test func aBodyThatIsNotAChatIsAnError() {
        #expect(throws: RequestError.self) { try request("nope") }
        #expect(throws: RequestError.self) { try request(#"{"messages":"x"}"#) }
        #expect(throws: RequestError.self) { try request(#"{"messages":[]}"#) }
        #expect(throws: RequestError.self) { try request(#"{"messages":[{"role":"user","content":"q"}],"tools":[{"type":"function"}]}"#) }
    }
}
