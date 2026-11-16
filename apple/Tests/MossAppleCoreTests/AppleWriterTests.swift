import Foundation
import FoundationModels
import Testing

@testable import MossAppleCore

/// These run Apple's own model, so only on a Mac where Apple Intelligence is on.
private let available = SystemLanguageModel.default.isAvailable

private func written(_ body: String) async throws -> [Piece] {
    let chat = try Chat(try ChatRequest.parse(Data(body.utf8)))
    var pieces: [Piece] = []
    try await AppleWriter().write(chat) { pieces.append($0) }
    return pieces
}

private let tools = #"""
[{"type":"function","function":{"name":"search_mail","description":"Find emails by words. Returns ids, subjects and senders.","parameters":{"type":"object","properties":{"query":{"type":"string","description":"Words to search for"}},"required":["query"]}}},
 {"type":"function","function":{"name":"mark_done","description":"Mark one email Done","parameters":{"type":"object","properties":{"thread_id":{"type":"string","description":"thread_id from search_mail"},"done":{"type":["boolean","null"]}},"required":["thread_id"]}}}]
"""#
private let system = #"{"role":"system","content":"You are Moss, an email assistant. Use the tools to act on the user's mail. Find emails with search_mail first and use the id it returns. Never ask the user for ids."}"#

@Suite(.enabled(if: available), .serialized) struct AppleWriterTests {
    @Test func writesAnAnswerInPieces() async throws {
        let pieces = try await written(#"{"messages":[{"role":"user","content":"Say hello in three words."}],"max_tokens":30,"temperature":0}"#)
        let text = pieces.map { if case .text(let t) = $0 { t } else { "" } }.joined()
        #expect(!text.isEmpty)
        #expect(!pieces.contains { if case .calls = $0 { true } else { false } })
    }

    @Test func callsATool() async throws {
        let pieces = try await written(#"{"messages":[\#(system),{"role":"user","content":"Mark the newsletter from Framely as done."}],"tools":\#(tools),"temperature":0}"#)
        guard case .calls(let calls) = try #require(pieces.last) else { Issue.record("no calls: \(pieces)"); return }
        #expect(calls.first?.name == "search_mail")
        #expect(calls.first?.arguments.lowercased().contains("framely") == true)
    }

    @Test func goesOnFromAToolsResult() async throws {
        let body = #"""
        {"messages":[\#(system),{"role":"user","content":"Mark the newsletter from Framely as done."},
          {"role":"assistant","content":"","tool_calls":[{"id":"c1","type":"function","function":{"name":"search_mail","arguments":"{\"query\":\"Framely\"}"}}]},
          {"role":"tool","tool_call_id":"c1","content":"[{\"thread_id\":\"41\",\"subject\":\"Config 2026 recap\",\"from\":\"Framely <news@framely.example>\"},{\"thread_id\":\"7\",\"subject\":\"Your Framely invoice\",\"from\":\"billing@framely.example\"}]"}],
         "tools":\#(tools),"temperature":0}
        """#
        let pieces = try await written(body)
        guard case .calls(let calls) = try #require(pieces.last) else { Issue.record("no calls: \(pieces)"); return }
        #expect(calls.map(\.name) == ["mark_done"])
        #expect(calls.first?.arguments.contains("41") == true)
    }

    @Test func callsAToolWhenItHasTo() async throws {
        let body = #"{"messages":[{"role":"user","content":"The user asked: mark Framely done. The answer: I marked it Done. Review it."}],"tools":[{"type":"function","function":{"name":"review","description":"Whether it is done","parameters":{"type":"object","properties":{"done":{"type":"boolean"},"next":{"type":"string"}},"required":["done","next"]}}}],"tool_choice":"required","temperature":0}"#
        let pieces = try await written(body)
        guard case .calls(let calls) = try #require(pieces.last) else { Issue.record("no calls: \(pieces)"); return }
        #expect(calls.first?.name == "review")
        let args = try JSONSerialization.jsonObject(with: Data(try #require(calls.first).arguments.utf8)) as? [String: Any]
        #expect(args?["done"] is Bool)
    }

    @Test func aChatTooLongForItIsSaidInPlainWords() async throws {
        // Plain, varied text: the same word over and over trips Apple's safety rules first.
        let long = (1...3_000).map { "Item \($0) is on the list for the spring shoot." }.joined(separator: " ")
        do {
            _ = try await written(#"{"messages":[{"role":"user","content":"\#(long)"}]}"#)
            Issue.record("it read it all")
        } catch {
            #expect(Failure.words(error) == "This is too long for Apple Intelligence to read at once.", "\(error)")
        }
    }

    @Test func answersInTheShapeAsked() async throws {
        let body = #"{"messages":[{"role":"user","content":"Is the sky blue on a clear day?"}],"response_format":{"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object","properties":{"yes":{"type":"boolean"}},"required":["yes"]}}},"temperature":0}"#
        let pieces = try await written(body)
        let text = pieces.map { if case .text(let t) = $0 { t } else { "" } }.joined()
        let json = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any]
        #expect(json?["yes"] as? Bool == true)
    }
}
