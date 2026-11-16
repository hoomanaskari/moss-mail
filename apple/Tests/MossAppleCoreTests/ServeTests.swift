import Foundation
import FoundationModels
import Testing

@testable import MossAppleCore

/// Says what it's told to, and remembers what it was asked.
private final class FakeWriter: Writer, @unchecked Sendable {
    var pieces: [Piece] = []
    var failure: (any Error)?
    var asked: [Chat] = []

    func write(_ chat: Chat, on piece: (Piece) async throws -> Void) async throws {
        asked.append(chat)
        for p in pieces { try await piece(p) }
        if let failure { throw failure }
    }
}

private func http(_ method: String, _ path: String, key: String? = "k1", body: String = "") -> HTTPRequest {
    var headers: [String: String] = [:]
    if let key { headers["authorization"] = "Bearer \(key)" }
    return HTTPRequest(method: method, path: path, headers: headers, body: Data(body.utf8))
}

private func run(_ handler: Handler, _ request: HTTPRequest) async -> String {
    var out = Data()
    await handler.respond(to: request) { out.append($0) }
    return String(decoding: out, as: UTF8.self)
}

private let ask = #"{"messages":[{"role":"user","content":"Hi"}]}"#

@Suite struct ServeTests {
    @Test func healthNeedsNoKey() async {
        let out = await run(Handler(key: "k1", writer: FakeWriter()), http("GET", "/health", key: nil))
        #expect(out.hasPrefix("HTTP/1.1 200 OK"))
        #expect(out.hasSuffix(#"{"status":"ok"}"#))
    }

    @Test func aChatNeedsTheKey() async {
        let writer = FakeWriter()
        for key in [nil, "wrong"] {
            let out = await run(Handler(key: "k1", writer: writer), http("POST", "/v1/chat/completions", key: key, body: ask))
            #expect(out.hasPrefix("HTTP/1.1 401"))
        }
        #expect(writer.asked.isEmpty)
    }

    @Test func anythingElseIsNotFound() async {
        let out = await run(Handler(key: "k1", writer: FakeWriter()), http("GET", "/v1/models"))
        #expect(out.hasPrefix("HTTP/1.1 404"))
    }

    @Test func aBodyThatIsNotAChatIsABadRequest() async {
        let out = await run(Handler(key: "k1", writer: FakeWriter()), http("POST", "/v1/chat/completions", body: "nope"))
        #expect(out.hasPrefix("HTTP/1.1 400"))
        #expect(out.contains("error"))
    }

    @Test func anAnswerStreamsAsItIsWritten() async {
        let writer = FakeWriter()
        writer.pieces = [.text("Hel"), .text("lo")]
        let out = await run(Handler(key: "k1", writer: writer), http("POST", "/v1/chat/completions", body: ask))
        #expect(out.hasPrefix("HTTP/1.1 200 OK"))
        #expect(out.contains("text/event-stream"))
        #expect(out.contains(#""content":"Hel""#))
        #expect(out.contains(#""content":"lo""#))
        #expect(out.contains(#""finish_reason":"stop""#))
        #expect(out.hasSuffix("data: [DONE]\n\n"))
        #expect(writer.asked.first?.prompt == "Hi")
    }

    @Test func toolCallsEndTheAnswer() async {
        let writer = FakeWriter()
        writer.pieces = [.calls([Call(id: "c1", name: "f", arguments: "{}")])]
        let out = await run(Handler(key: "k1", writer: writer), http("POST", "/v1/chat/completions", body: ask))
        #expect(out.contains(#""tool_calls""#))
        #expect(out.contains(#""finish_reason":"tool_calls""#))
    }

    @Test func aFailureWhileWritingIsAnErrorEvent() async {
        let writer = FakeWriter()
        writer.pieces = [.text("Hi")]
        writer.failure = LanguageModelError.contextSizeExceeded(.init(contextSize: 8192, tokenCount: 9000, debugDescription: "big"))
        let out = await run(Handler(key: "k1", writer: writer), http("POST", "/v1/chat/completions", body: ask))
        #expect(out.contains(#""content":"Hi""#))
        #expect(out.contains(#"{"error":{"message":"#))
        #expect(out.contains("too long for Apple Intelligence"))
        #expect(!out.contains("finish_reason"))
        #expect(out.hasSuffix("data: [DONE]\n\n"))
    }

    @Test func aToolWhoseSchemaAppleCannotReadIsAnErrorEvent() async {
        let body = #"{"messages":[{"role":"user","content":"q"}],"tools":[{"type":"function","function":{"name":"bad","parameters":{"type":"object","properties":{"a":{"type":"nonsense"}}}}}]}"#
        let out = await run(Handler(key: "k1", writer: FakeWriter()), http("POST", "/v1/chat/completions", body: body))
        #expect(out.hasPrefix("HTTP/1.1 200 OK"))
        #expect(out.contains("couldn’t read the tool bad"))
    }

    @Test func aGoneListenerStopsTheAnswer() async {
        let writer = FakeWriter()
        writer.pieces = [.text("a"), .text("b")]
        var writes = 0
        struct Gone: Error {}
        await Handler(key: "k1", writer: writer).respond(to: http("POST", "/v1/chat/completions", body: ask)) { _ in
            writes += 1
            if writes == 2 { throw Gone() }
        }
        // The head, then the first piece fails: nothing more is written.
        #expect(writes == 2)
    }
}

@Suite struct FailureTests {
    typealias E = LanguageModelError

    @Test func eachFailureIsSaidInPlainWords() {
        #expect(Failure.words(E.contextSizeExceeded(.init(contextSize: 8192, tokenCount: 9000, debugDescription: "x"))).contains("too long"))
        #expect(Failure.words(E.guardrailViolation(.init(debugDescription: "x"))).contains("safety"))
        #expect(Failure.words(E.refusal(.init(explanation: "no", debugDescription: "x"))).contains("wouldn’t write"))
        #expect(Failure.words(E.unsupportedLanguageOrLocale(.init(languageCode: .init("tlh"), debugDescription: "x"))).contains("language"))
        #expect(Failure.words(E.rateLimited(.init(resetDate: nil, debugDescription: "x"))).contains("busy"))
        #expect(Failure.words(E.timeout(.init(debugDescription: "x"))).contains("too long to answer"))
        #expect(Failure.words(E.unsupportedGenerationGuide(.init(schemaName: nil, debugDescription: "x"))).hasPrefix("Apple Intelligence couldn’t answer"))
        struct Other: Error {}
        #expect(Failure.words(Other()).hasPrefix("Apple Intelligence couldn’t answer"))
    }
}

@Suite struct AvailabilityTests {
    @Test func saysWhetherAppleIntelligenceCanRunAndWhyNot() {
        #expect(Availability.json(.available, context: 8192) == #"{"available":true,"context":8192}"#)
        #expect(Availability.json(.unavailable(.appleIntelligenceNotEnabled), context: 0) == #"{"available":false,"reason":"notEnabled"}"#)
        #expect(Availability.json(.unavailable(.deviceNotEligible), context: 0) == #"{"available":false,"reason":"notEligible"}"#)
        #expect(Availability.json(.unavailable(.modelNotReady), context: 0) == #"{"available":false,"reason":"notReady"}"#)
    }

    @Test func readsThisMac() async {
        let json = await Availability.json()
        #expect(json.hasPrefix(#"{"available":"#))
    }
}
