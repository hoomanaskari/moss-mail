import Foundation

/// The server-sent events of an OpenAI-style streamed answer.
public enum Stream {
    private static func event(_ json: [String: Any]) -> Data {
        let body = try! JSONSerialization.data(withJSONObject: json, options: [.sortedKeys, .withoutEscapingSlashes])
        return Data("data: ".utf8) + body + Data("\n\n".utf8)
    }

    private static func chunk(_ delta: [String: Any], finish: String? = nil) -> Data {
        var choice: [String: Any] = ["index": 0, "delta": delta]
        if let finish { choice["finish_reason"] = finish }
        return event(["object": "chat.completion.chunk", "model": "apple", "choices": [choice]])
    }

    static func text(_ text: String) -> Data {
        chunk(["content": text])
    }

    static func calls(_ calls: [Call]) -> Data {
        let list = calls.enumerated().map { i, call in
            ["index": i, "id": call.id, "type": "function", "function": ["name": call.name, "arguments": call.arguments]] as [String: Any]
        }
        return chunk(["tool_calls": list])
    }

    static func finish(_ reason: String) -> Data {
        chunk([:], finish: reason)
    }

    static func error(_ message: String) -> Data {
        event(["error": ["message": message]])
    }

    static let done = Data("data: [DONE]\n\n".utf8)

    /// What a growing answer has that wasn't sent yet. Apple's model gives the whole answer so
    /// far each time; what was sent can't be taken back, so only what's beyond it counts.
    static func added(sent: String, now: String) -> String {
        if now.hasPrefix(sent) { return String(now.dropFirst(sent.count)) }
        return String(now.dropFirst(min(sent.count, now.count)))
    }
}

/// An HTTP request, read from the connection.
public struct HTTPRequest: Equatable {
    var method: String
    var path: String
    /// Names in lower case.
    var headers: [String: String]
    var body: Data

    /// Bodies bigger than this aren't chats.
    static let most = 16 << 20

    /// The request, once all of it is in; nil while some is still to come.
    public static func parse(_ data: Data) throws -> HTTPRequest? {
        guard let end = data.range(of: Data("\r\n\r\n".utf8)) else { return nil }
        let head = String(decoding: data[..<end.lowerBound], as: UTF8.self).components(separatedBy: "\r\n")
        let start = head[0].split(separator: " ")
        guard start.count == 3, start[2].hasPrefix("HTTP/") else { throw RequestError("Not an HTTP request.") }
        var headers: [String: String] = [:]
        for line in head.dropFirst() {
            guard let colon = line.firstIndex(of: ":") else { continue }
            headers[line[..<colon].lowercased()] = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
        }
        let length = headers["content-length"].map { Int($0) } ?? 0
        guard let length, length <= most else { throw RequestError("A body Moss can’t read.") }
        let body = data[end.upperBound...]
        guard body.count >= length else { return nil }
        return HTTPRequest(method: String(start[0]), path: String(start[1]), headers: headers, body: Data(body.prefix(length)))
    }
}

/// HTTP responses: the head of a stream, or a whole small answer.
public enum HTTPResponse {
    private static func reason(_ status: Int) -> String {
        switch status {
        case 200: "OK"
        case 400: "Bad Request"
        case 401: "Unauthorized"
        case 404: "Not Found"
        default: "Error"
        }
    }

    static func head(_ status: Int, type: String, length: Int? = nil) -> Data {
        var head = "HTTP/1.1 \(status) \(reason(status))\r\nContent-Type: \(type)\r\nCache-Control: no-cache\r\nConnection: close\r\n"
        if let length { head += "Content-Length: \(length)\r\n" }
        return Data((head + "\r\n").utf8)
    }

    public static func whole(_ status: Int, json: String) -> Data {
        head(status, type: "application/json", length: json.utf8.count) + Data(json.utf8)
    }
}

/// What the helper was started with. It takes llama-server's flags and uses the ones it needs.
public struct Options {
    public var host = "127.0.0.1"
    public var port: Int?
    public var keyFile = ""
    /// Only say whether Apple Intelligence can run, then quit.
    public var check = false

    public static func parse(_ arguments: [String]) throws -> Options {
        var options = Options()
        var rest = arguments[...]
        func value(_ flag: String) throws -> String {
            guard let v = rest.popFirst() else { throw RequestError("\(flag) needs a value.") }
            return v
        }
        while let flag = rest.popFirst() {
            switch flag {
            case "--available": options.check = true
            case "--host": options.host = try value(flag)
            case "--port":
                guard let port = Int(try value(flag)) else { throw RequestError("--port needs a number.") }
                options.port = port
            case "--api-key-file": options.keyFile = try value(flag)
            default: break
            }
        }
        if !options.check && (options.port == nil || options.keyFile.isEmpty) {
            throw RequestError("moss-apple needs --port and --api-key-file, or --available.")
        }
        return options
    }
}
