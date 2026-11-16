import Foundation
import Testing

@testable import MossAppleCore

private func event(_ data: Data) -> [String: Any] {
    let line = String(decoding: data, as: UTF8.self)
    #expect(line.hasPrefix("data: "))
    #expect(line.hasSuffix("\n\n"))
    let json = line.dropFirst(6).trimmingCharacters(in: .whitespacesAndNewlines)
    return try! JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any]
}

private func delta(_ data: Data) -> [String: Any] {
    let choices = event(data)["choices"] as! [[String: Any]]
    return choices[0]["delta"] as! [String: Any]
}

@Suite struct StreamTests {
    @Test func textGoesOutAsAChunkOfTheAnswer() {
        let e = event(Stream.text("Hi \"you\"\n"))
        #expect(e["object"] as? String == "chat.completion.chunk")
        #expect(e["model"] as? String == "apple")
        #expect(delta(Stream.text("Hi \"you\"\n"))["content"] as? String == "Hi \"you\"\n")
    }

    @Test func toolCallsGoOutInTheOpenAIShape() {
        let d = delta(Stream.calls([Call(id: "c1", name: "search_mail", arguments: #"{"query":"q"}"#), Call(id: "c2", name: "mark_done", arguments: "{}")]))
        let calls = d["tool_calls"] as! [[String: Any]]
        #expect(calls.count == 2)
        #expect(calls[1]["index"] as? Int == 1)
        #expect(calls[0]["id"] as? String == "c1")
        #expect(calls[0]["type"] as? String == "function")
        let function = calls[0]["function"] as! [String: Any]
        #expect(function["name"] as? String == "search_mail")
        #expect(function["arguments"] as? String == #"{"query":"q"}"#)
    }

    @Test func theEndSaysWhy() {
        let e = event(Stream.finish("tool_calls"))
        let choice = (e["choices"] as! [[String: Any]])[0]
        #expect(choice["finish_reason"] as? String == "tool_calls")
        #expect(String(decoding: Stream.done, as: UTF8.self) == "data: [DONE]\n\n")
    }

    @Test func aFailureIsAnErrorEvent() {
        let e = event(Stream.error("Apple Intelligence is off."))
        #expect((e["error"] as? [String: Any])?["message"] as? String == "Apple Intelligence is off.")
    }

    @Test func theNewPartOfAGrowingAnswerIsWhatIsAfterWhatWasSent() {
        #expect(Stream.added(sent: "Hel", now: "Hello") == "lo")
        #expect(Stream.added(sent: "", now: "Hi") == "Hi")
        #expect(Stream.added(sent: "Hello", now: "Hello") == "")
        // A rewrite of what was sent can't be taken back: only what's past it goes out.
        #expect(Stream.added(sent: "Hello", now: "Help me") == "me")
        #expect(Stream.added(sent: "Hello there", now: "Hi") == "")
    }
}

@Suite struct HTTPTests {
    private let post = "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer k1\r\nContent-Length: 2\r\n\r\n{}"

    @Test func readsARequestOnceItIsAllIn() throws {
        let r = try #require(try HTTPRequest.parse(Data(post.utf8)))
        #expect(r.method == "POST")
        #expect(r.path == "/v1/chat/completions")
        #expect(r.headers["authorization"] == "Bearer k1")
        #expect(r.body == Data("{}".utf8))
    }

    @Test func waitsForTheRestOfARequest() throws {
        #expect(try HTTPRequest.parse(Data("POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\n{}".utf8)) == nil)
        #expect(try HTTPRequest.parse(Data("GET /health HTTP/1.1\r\nHost: x".utf8)) == nil)
    }

    @Test func aHeaderLineWithNoColonIsSkipped() throws {
        let r = try #require(try HTTPRequest.parse(Data("GET /health HTTP/1.1\r\nnonsense\r\nHost: x\r\n\r\n".utf8)))
        #expect(r.headers == ["host": "x"])
    }

    @Test func aRequestWithNoBodyIsWhole() throws {
        let r = try #require(try HTTPRequest.parse(Data("GET /health HTTP/1.1\r\n\r\n".utf8)))
        #expect(r.method == "GET")
        #expect(r.body.isEmpty)
    }

    @Test func aBrokenRequestIsAnError() {
        #expect(throws: RequestError.self) { try HTTPRequest.parse(Data("nonsense\r\n\r\n".utf8)) }
        #expect(throws: RequestError.self) { try HTTPRequest.parse(Data("POST / HTTP/1.1\r\nContent-Length: lots\r\n\r\n".utf8)) }
        #expect(throws: RequestError.self) { try HTTPRequest.parse(Data("POST / HTTP/1.1\r\nContent-Length: 99999999\r\n\r\n".utf8)) }
    }

    @Test func aResponseHeadNamesItsStatusAndType() {
        let head = String(decoding: HTTPResponse.head(200, type: "text/event-stream"), as: UTF8.self)
        #expect(head.hasPrefix("HTTP/1.1 200 OK\r\n"))
        #expect(head.contains("Content-Type: text/event-stream\r\n"))
        #expect(head.contains("Connection: close\r\n"))
        #expect(head.hasSuffix("\r\n\r\n"))
        let whole = String(decoding: HTTPResponse.whole(401, json: #"{"error":"x"}"#), as: UTF8.self)
        #expect(whole.hasPrefix("HTTP/1.1 401 Unauthorized\r\n"))
        #expect(whole.contains("Content-Length: 13\r\n"))
        #expect(whole.hasSuffix(#"{"error":"x"}"#))
        #expect(String(decoding: HTTPResponse.head(404, type: "x"), as: UTF8.self).hasPrefix("HTTP/1.1 404 Not Found"))
        #expect(String(decoding: HTTPResponse.head(400, type: "x"), as: UTF8.self).hasPrefix("HTTP/1.1 400 Bad Request"))
        #expect(String(decoding: HTTPResponse.head(503, type: "x"), as: UTF8.self).hasPrefix("HTTP/1.1 503 Error"))
    }
}

@Suite struct OptionsTests {
    @Test func takesWhatLlamaServerTakesAndLeavesTheRest() throws {
        let o = try Options.parse(["-m", "apple", "--host", "127.0.0.1", "--port", "5123", "--no-webui", "--api-key-file", "/tmp/k", "-np", "1", "-c", "32768"])
        #expect(o.host == "127.0.0.1")
        #expect(o.port == 5123)
        #expect(o.keyFile == "/tmp/k")
        #expect(o.check == false)
    }

    @Test func anAvailabilityCheckNeedsNoPort() throws {
        let o = try Options.parse(["--available"])
        #expect(o.check)
        #expect(o.port == nil)
        #expect(o.host == "127.0.0.1")
    }

    @Test func aServerNeedsAPortAndAKey() {
        #expect(throws: RequestError.self) { try Options.parse(["--api-key-file", "/tmp/k"]) }
        #expect(throws: RequestError.self) { try Options.parse(["--port", "5123"]) }
        #expect(throws: RequestError.self) { try Options.parse(["--port", "lots", "--api-key-file", "k"]) }
        #expect(throws: RequestError.self) { try Options.parse(["--port"]) }
        #expect(throws: RequestError.self) { try Options.parse(["--available", "--host"]) }
    }
}
