import Foundation
import FoundationModels

/// The tool calls the model asked for in one answer. Moss runs them, not this helper, so a call
/// is written down here and the answer stops.
final class Heard: Error, @unchecked Sendable {
    private let lock = NSLock()
    private var list: [Call] = []

    var calls: [Call] { lock.withLock { list } }

    func add(_ call: Call) {
        lock.withLock { list.append(call) }
    }
}

/// A tool Apple's model can call, from a JSON schema. Calling it only writes the call down.
struct HeardTool: Tool {
    let name: String
    let description: String
    let parameters: GenerationSchema
    let heard: Heard

    func call(arguments: GeneratedContent) async throws -> String {
        heard.add(Call(id: "call_\(UUID().uuidString.prefix(8).lowercased())", name: name, arguments: arguments.jsonString))
        throw heard
    }
}

/// A request as Apple's model takes it: the chat so far as a transcript, the prompt to answer,
/// the tools, and how to write.
public struct Chat {
    var transcript: Transcript
    var tools: [any Tool]
    var prompt: String
    var options: GenerationOptions
    var format: GenerationSchema?
    let heard = Heard()

    public init(_ request: ChatRequest) throws {
        let heard = heard
        tools = try request.tools.map { spec in
            do {
                let parameters = try Schema.generation(spec.parameters, title: spec.name)
                return HeardTool(name: spec.name, description: spec.description, parameters: parameters, heard: heard)
            } catch {
                throw RequestError("Apple Intelligence couldn’t read the tool \(spec.name).")
            }
        }
        let definitions = tools.map { Transcript.ToolDefinition(tool: $0) }
        var entries: [Transcript.Entry] = [.instructions(.init(segments: Self.words(request.instructions), toolDefinitions: definitions))]
        for turn in request.history {
            switch turn {
            case .user(let text):
                entries.append(.prompt(.init(segments: Self.words(text))))
            case .assistant(let text):
                entries.append(.response(.init(assetIDs: [], segments: Self.words(text))))
            case .calls(let calls):
                entries.append(.toolCalls(.init(calls.map { call in
                    let arguments = (try? GeneratedContent(json: call.arguments)) ?? GeneratedContent(properties: [:])
                    return Transcript.ToolCall(id: call.id, toolName: call.name, arguments: arguments)
                })))
            case .output(let id, let name, let text):
                entries.append(.toolOutput(.init(id: id, toolName: name, segments: Self.words(text))))
            }
        }
        transcript = Transcript(entries: entries)
        prompt = request.prompt
        var options = GenerationOptions(temperature: request.temperature, maximumResponseTokens: request.maxTokens)
        if request.temperature == 0 { options.samplingMode = .greedy }
        if request.mustCall && !tools.isEmpty { options.toolCallingMode = .required }
        self.options = options
        if let shape = request.format, tools.isEmpty {
            format = try? Schema.generation(shape.schema, title: shape.name)
        }
    }

    private static func words(_ text: String) -> [Transcript.Segment] {
        [.text(.init(content: text))]
    }
}

/// A piece of an answer: words as they're written, or the tool calls that end it.
public enum Piece: Equatable {
    case text(String)
    case calls([Call])
}

/// Writes answers. The helper uses [`AppleWriter`]; tests pass a fake.
public protocol Writer: Sendable {
    func write(_ chat: Chat, on piece: (Piece) async throws -> Void) async throws
}

/// Apple's on-device model.
public struct AppleWriter: Writer {
    public init() {}

    public func write(_ chat: Chat, on piece: (Piece) async throws -> Void) async throws {
        let session = LanguageModelSession(tools: chat.tools, transcript: chat.transcript)
        do {
            if let format = chat.format {
                let answer = try await session.respond(to: chat.prompt, schema: format, options: chat.options)
                return try await piece(.text(answer.content.jsonString))
            }
            var sent = ""
            for try await snapshot in session.streamResponse(to: chat.prompt, options: chat.options) {
                let added = Stream.added(sent: sent, now: snapshot.content)
                if !added.isEmpty {
                    sent += added
                    try await piece(.text(added))
                }
            }
        } catch let error as LanguageModelSession.ToolCallError where error.underlyingError is Heard {
            try await piece(.calls(chat.heard.calls))
        }
    }
}
