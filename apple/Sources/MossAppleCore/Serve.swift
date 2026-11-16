import Foundation
import FoundationModels

/// Answers one HTTP request: `/health`, or a chat on `/v1/chat/completions` behind the run's key.
public struct Handler: Sendable {
    let key: String
    let writer: any Writer

    public init(key: String, writer: any Writer) {
        self.key = key
        self.writer = writer
    }

    /// Writes the response through `write`. When `write` fails the listener has gone, so the
    /// answer stops there.
    public func respond(to request: HTTPRequest, write: (Data) async throws -> Void) async {
        do {
            switch (request.method, request.path) {
            case ("GET", "/health"):
                return try await write(HTTPResponse.whole(200, json: #"{"status":"ok"}"#))
            case ("POST", "/v1/chat/completions"):
                break
            default:
                return try await write(HTTPResponse.whole(404, json: #"{"error":{"message":"Not found."}}"#))
            }
            guard request.headers["authorization"] == "Bearer \(key)" else {
                return try await write(HTTPResponse.whole(401, json: #"{"error":{"message":"Wrong key."}}"#))
            }
            let parsed: ChatRequest
            do {
                parsed = try ChatRequest.parse(request.body)
            } catch let error as RequestError {
                return try await write(HTTPResponse.whole(400, json: Self.error(error.message)))
            }
            try await write(HTTPResponse.head(200, type: "text/event-stream"))
            try await stream(parsed, write: write)
            try await write(Stream.done)
        } catch {
            // The listener went away.
        }
    }

    private func stream(_ request: ChatRequest, write: (Data) async throws -> Void) async throws {
        let chat: Chat
        do {
            chat = try Chat(request)
        } catch let error as RequestError {
            return try await write(Stream.error(error.message))
        }
        var finish = "stop"
        var gone: (any Error)?
        do {
            try await writer.write(chat) { piece in
                do {
                    switch piece {
                    case .text(let text): try await write(Stream.text(text))
                    case .calls(let calls):
                        finish = "tool_calls"
                        try await write(Stream.calls(calls))
                    }
                } catch {
                    gone = error
                    throw error
                }
            }
        } catch {
            if let gone { throw gone }
            return try await write(Stream.error(Failure.words(error)))
        }
        try await write(Stream.finish(finish))
    }

    private static func error(_ message: String) -> String {
        let body = try! JSONSerialization.data(withJSONObject: ["error": ["message": message]])
        return String(decoding: body, as: UTF8.self)
    }
}

/// Why Apple's model didn't answer, in words Moss can show.
public enum Failure {
    public static func words(_ error: any Error) -> String {
        guard let error = error as? LanguageModelError else {
            return "Apple Intelligence couldn’t answer: \(error.localizedDescription)"
        }
        switch error {
        case .contextSizeExceeded:
            return "This is too long for Apple Intelligence to read at once."
        case .guardrailViolation:
            return "Apple Intelligence stopped: its safety rules wouldn’t let it write about this."
        case .refusal:
            return "Apple Intelligence wouldn’t write about this."
        case .unsupportedLanguageOrLocale:
            return "Apple Intelligence doesn’t write in this language yet."
        case .rateLimited:
            return "Apple Intelligence is busy. Try again in a moment."
        case .timeout:
            return "Apple Intelligence took too long to answer. Try again."
        default:
            return "Apple Intelligence couldn’t answer: \(error.localizedDescription)"
        }
    }
}

/// Whether Apple Intelligence can run on this Mac, as one line of JSON for Moss.
public enum Availability {
    static func json(_ availability: SystemLanguageModel.Availability, context: Int) -> String {
        switch availability {
        case .available:
            return #"{"available":true,"context":\#(context)}"#
        case .unavailable(let reason):
            let why = switch reason {
            case .appleIntelligenceNotEnabled: "notEnabled"
            case .deviceNotEligible: "notEligible"
            default: "notReady"
            }
            return #"{"available":false,"reason":"\#(why)"}"#
        }
    }

    public static func json() async -> String {
        let model = SystemLanguageModel.default
        return json(model.availability, context: model.contextSize)
    }
}
