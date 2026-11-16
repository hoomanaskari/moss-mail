// moss-apple: Apple's on-device model behind an OpenAI-style chat endpoint on 127.0.0.1, so
// Moss starts and talks to it the way it does llama-server:
//
//     moss-apple --host 127.0.0.1 --port 52011 --api-key-file <file>   serve
//     moss-apple --available                                          say whether it can run
//
// What it does with each request is in MossAppleCore, where the tests reach it. This file only
// listens and passes bytes along.

import Foundation
import FoundationModels
import MossAppleCore
import Network

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(2)
}

let options: Options
do {
    options = try Options.parse(Array(CommandLine.arguments.dropFirst()))
} catch {
    fail((error as? RequestError)?.message ?? "\(error)")
}

if options.check {
    print(await Availability.json())
    exit(0)
}

guard let key = try? String(contentsOfFile: options.keyFile, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines), !key.isEmpty else {
    fail("Can’t read the key in \(options.keyFile).")
}
let handler = Handler(key: key, writer: AppleWriter())

/// Sends `data`, and waits until it's gone.
func send(_ data: Data, on connection: NWConnection) async throws {
    try await withCheckedThrowingContinuation { (done: CheckedContinuation<Void, any Error>) in
        connection.send(content: data, completion: .contentProcessed { error in
            if let error { done.resume(throwing: error) } else { done.resume() }
        })
    }
}

/// The next bytes from the connection; empty once it's closed.
func receive(on connection: NWConnection) async throws -> Data {
    try await withCheckedThrowingContinuation { (done: CheckedContinuation<Data, any Error>) in
        connection.receive(minimumIncompleteLength: 1, maximumLength: 1 << 16) { data, _, finished, error in
            if let error { return done.resume(throwing: error) }
            done.resume(returning: data ?? Data())
        }
    }
}

/// Reads one request, answers it, and closes the connection.
func serve(_ connection: NWConnection) async {
    connection.start(queue: .global())
    defer { connection.cancel() }
    var buffer = Data()
    do {
        while true {
            if let request = try HTTPRequest.parse(buffer) {
                return await handler.respond(to: request) { try await send($0, on: connection) }
            }
            let more = try await receive(on: connection)
            if more.isEmpty { return }
            buffer.append(more)
        }
    } catch {
        try? await send(HTTPResponse.whole(400, json: #"{"error":{"message":"Bad request."}}"#), on: connection)
    }
}

let parameters = NWParameters.tcp
guard let port = options.port.flatMap({ NWEndpoint.Port(rawValue: UInt16(clamping: $0)) }) else { fail("No port.") }
parameters.requiredLocalEndpoint = .hostPort(host: NWEndpoint.Host(options.host), port: port)
parameters.allowLocalEndpointReuse = true
let listener: NWListener
do {
    listener = try NWListener(using: parameters)
} catch {
    fail("Can’t listen on \(options.host):\(port): \(error)")
}
listener.newConnectionHandler = { connection in
    Task { await serve(connection) }
}
listener.stateUpdateHandler = { state in
    if case .failed(let error) = state { fail("Stopped listening: \(error)") }
}
listener.start(queue: .global())

// The first answer comes sooner with the model already loaded.
LanguageModelSession().prewarm()

// Serve until Moss stops this process.
while true {
    try? await Task.sleep(for: .seconds(3600))
}
