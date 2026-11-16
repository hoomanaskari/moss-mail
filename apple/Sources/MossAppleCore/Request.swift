import Foundation

/// A request Moss couldn't make sense of.
public struct RequestError: Error, Equatable {
    public var message: String
    init(_ message: String) { self.message = message }
}

/// A tool call: its id, the tool's name, and its arguments as JSON text.
public struct Call: Equatable, Sendable {
    public var id: String
    public var name: String
    public var arguments: String
}

/// One turn of the chat before the one being answered.
public enum Turn: Equatable {
    case user(String)
    case assistant(String)
    case calls([Call])
    case output(id: String, name: String, text: String)
}

/// A tool on offer, with its arguments' JSON schema.
public struct ToolSpec {
    public var name: String
    public var description: String
    public var parameters: Any
}

/// The JSON shape an answer must have.
public struct Format {
    public var name: String
    public var schema: Any
}

/// An OpenAI-style chat request, as Moss's Rust side sends it to llama-server.
public struct ChatRequest {
    /// Every system message, in order.
    public var instructions = ""
    public var history: [Turn] = []
    /// The last user message; empty when the model goes on from a tool's result or its own words.
    public var prompt = ""
    public var tools: [ToolSpec] = []
    /// The model has to call one of the tools (`tool_choice: "required"`).
    public var mustCall = false
    public var format: Format?
    public var maxTokens: Int?
    public var temperature: Double?

    public static func parse(_ body: Data) throws -> ChatRequest {
        guard let json = try? JSONSerialization.jsonObject(with: body) as? [String: Any] else {
            throw RequestError("The request isn’t JSON.")
        }
        guard let messages = json["messages"] as? [[String: Any]], !messages.isEmpty else {
            throw RequestError("The request has no messages.")
        }
        var request = ChatRequest()
        var system: [String] = []
        var names: [String: String] = [:]
        for message in messages {
            let content = text(message["content"])
            switch message["role"] as? String {
            case "system", "developer":
                system.append(content)
            case "assistant":
                let calls = (message["tool_calls"] as? [[String: Any]] ?? []).map { call in
                    let function = call["function"] as? [String: Any] ?? [:]
                    return Call(id: call["id"] as? String ?? "", name: function["name"] as? String ?? "", arguments: function["arguments"] as? String ?? "{}")
                }
                if !content.isEmpty { request.history.append(.assistant(content)) }
                if !calls.isEmpty {
                    calls.forEach { names[$0.id] = $0.name }
                    request.history.append(.calls(calls))
                }
            case "tool":
                let id = message["tool_call_id"] as? String ?? ""
                request.history.append(.output(id: id, name: names[id] ?? "", text: content))
            default:
                request.history.append(.user(content))
            }
        }
        if case .user(let last) = request.history.last {
            request.history.removeLast()
            request.prompt = last
        }
        request.instructions = system.joined(separator: "\n\n")
        for tool in json["tools"] as? [[String: Any]] ?? [] {
            guard let function = tool["function"] as? [String: Any], let name = function["name"] as? String else {
                throw RequestError("A tool has no name.")
            }
            let parameters = function["parameters"] ?? ["type": "object", "properties": [String: Any]()]
            request.tools.append(ToolSpec(name: name, description: function["description"] as? String ?? "", parameters: parameters))
        }
        request.mustCall = json["tool_choice"] as? String == "required"
        if let format = json["response_format"] as? [String: Any], let shape = format["json_schema"] as? [String: Any], let schema = shape["schema"] {
            request.format = Format(name: shape["name"] as? String ?? "answer", schema: schema)
        }
        request.maxTokens = json["max_tokens"] as? Int
        request.temperature = (json["temperature"] as? NSNumber)?.doubleValue
        return request
    }

    /// A message's words: plain text, or the text parts of a list of parts.
    private static func text(_ content: Any?) -> String {
        if let text = content as? String { return text }
        let parts = content as? [[String: Any]] ?? []
        return parts.compactMap { $0["type"] as? String == "text" ? $0["text"] as? String : nil }.joined(separator: "\n")
    }
}
