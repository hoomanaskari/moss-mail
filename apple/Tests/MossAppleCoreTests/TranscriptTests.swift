import Foundation
import FoundationModels
import Testing

@testable import MossAppleCore

private func text(_ segments: [Transcript.Segment]) -> String {
    segments.compactMap { if case .text(let t) = $0 { t.content } else { nil } }.joined()
}

private func chat(_ body: String) throws -> Chat {
    try Chat(try ChatRequest.parse(Data(body.utf8)))
}

@Suite struct TranscriptTests {
    @Test func theInstructionsCarryTheToolsOnOffer() throws {
        let c = try chat(#"""
        {"messages":[{"role":"system","content":"Be brief."},{"role":"user","content":"Hi"}],
         "tools":[{"type":"function","function":{"name":"search_mail","description":"Find emails","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}}]}
        """#)
        let entries = Array(c.transcript)
        #expect(entries.count == 1)
        guard case .instructions(let instructions) = entries[0] else { Issue.record("no instructions"); return }
        #expect(text(instructions.segments) == "Be brief.")
        #expect(instructions.toolDefinitions.map(\.name) == ["search_mail"])
        #expect(c.tools.map(\.name) == ["search_mail"])
        #expect(c.prompt == "Hi")
    }

    @Test func historyBecomesTheTranscriptInOrder() throws {
        let c = try chat(#"""
        {"messages":[{"role":"user","content":"Q1"},{"role":"assistant","content":"A1"},{"role":"user","content":"Mark Framely done"},
          {"role":"assistant","content":"","tool_calls":[{"id":"c1","function":{"name":"search_mail","arguments":"{\"query\":\"Framely\"}"}}]},
          {"role":"tool","tool_call_id":"c1","content":"[t:41] Config recap"}]}
        """#)
        let entries = Array(c.transcript)
        #expect(entries.count == 6)
        guard case .instructions(let i) = entries[0], case .prompt(let q1) = entries[1], case .response(let a1) = entries[2],
            case .prompt(let q2) = entries[3], case .toolCalls(let calls) = entries[4], case .toolOutput(let out) = entries[5]
        else { Issue.record("entries out of order: \(entries)"); return }
        #expect(text(i.segments) == "")
        #expect(text(q1.segments) == "Q1")
        #expect(text(a1.segments) == "A1")
        #expect(text(q2.segments) == "Mark Framely done")
        #expect(calls.map(\.toolName) == ["search_mail"])
        #expect(calls.map(\.id) == ["c1"])
        #expect(calls.first?.arguments.jsonString.contains("Framely") == true)
        #expect(out.id == "c1")
        #expect(out.toolName == "search_mail")
        #expect(text(out.segments) == "[t:41] Config recap")
        #expect(c.prompt == "")
    }

    @Test func argumentsThatAreNotJSONReachTheModelAsNone() throws {
        let c = try chat(#"{"messages":[{"role":"user","content":"q"},{"role":"assistant","tool_calls":[{"id":"c1","function":{"name":"f","arguments":"{oops"}}]},{"role":"tool","tool_call_id":"c1","content":"r"}]}"#)
        guard case .toolCalls(let calls) = Array(c.transcript)[2] else { Issue.record("no calls"); return }
        #expect(calls.first?.arguments.jsonString.contains("oops") == false)
    }

    @Test func aToolWhoseSchemaAppleCannotReadIsAnError() {
        #expect(throws: (any Error).self) {
            try chat(#"{"messages":[{"role":"user","content":"q"}],"tools":[{"type":"function","function":{"name":"bad","parameters":{"type":"object","properties":{"a":{"type":"nonsense"}}}}}]}"#)
        }
    }

    @Test func theOptionsFollowTheRequest() throws {
        let c = try chat(#"{"messages":[{"role":"user","content":"q"}],"max_tokens":80,"temperature":0}"#)
        #expect(c.options.maximumResponseTokens == 80)
        #expect(c.options.temperature == 0)
        let free = try chat(#"{"messages":[{"role":"user","content":"q"}],"temperature":0.7}"#)
        #expect(free.options.temperature == 0.7)
        #expect(free.options.maximumResponseTokens == nil)
    }

    @Test func aRequiredCallIsRequired() throws {
        let c = try chat(#"{"messages":[{"role":"user","content":"q"}],"tools":[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{}}}}],"tool_choice":"required"}"#)
        #expect(c.options.toolCallingMode == .required)
        let free = try chat(#"{"messages":[{"role":"user","content":"q"}]}"#)
        #expect(free.options.toolCallingMode == nil)
    }

    @Test func aToolCallIsHeardNotRun() async throws {
        let c = try chat(#"{"messages":[{"role":"user","content":"q"}],"tools":[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{"a":{"type":"string"}}}}}]}"#)
        let tool = try #require(c.tools.first as? HeardTool)
        await #expect(throws: Heard.self) { _ = try await tool.call(arguments: try GeneratedContent(json: #"{"a":"x"}"#)) }
        #expect(c.heard.calls.map(\.name) == ["f"])
        #expect(c.heard.calls.first?.arguments.contains("\"a\"") == true)
        #expect(c.heard.calls.first?.id.isEmpty == false)
    }
}
