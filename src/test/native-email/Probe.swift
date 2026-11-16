import AppKit
import WebKit
struct Fixture: Decodable { let name: String; let html: String; let allowed: Bool; let sandbox: String }
final class Probe: NSObject, WKNavigationDelegate {
    let fixtures: [Fixture]
    let origin: String
    let view: WKWebView
    var index = 0
    var loading = false
    var observations: [[String: Any]] = []
    init(fixtures: [Fixture], origin: String) {
        self.fixtures = fixtures; self.origin = origin
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        view = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 600), configuration: config)
        super.init(); view.navigationDelegate = self
    }
    func run() {
        guard index < fixtures.count else {
            let data = try! JSONSerialization.data(withJSONObject: observations, options: [.sortedKeys])
            print(String(data: data, encoding: .utf8)!); exit(0)
        }
        loading = true
        view.loadHTMLString("<html><body><iframe id='email' sandbox='' style='width:700px;height:500px'></iframe></body></html>", baseURL: URL(string: origin))
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard loading else { return }; loading = false
        let fixture = fixtures[index]
        let html = fixture.html.replacingOccurrences(of: "http://127.0.0.1:12345", with: origin).replacingOccurrences(of: "https://127.0.0.1:12345", with: origin)
        let encoded = String(data: try! JSONSerialization.data(withJSONObject: [fixture.sandbox, html]), encoding: .utf8)!
        webView.evaluateJavaScript("window.compromised=false; document.getElementById('email').setAttribute('sandbox',(\(encoded))[0]); document.getElementById('email').srcdoc=(\(encoded))[1]") { _, error in
            guard error == nil else { print("SETUP FAILED: \(String(describing: error))"); exit(1) }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.observe() }
        }
    }
    func observe() {
        view.evaluateJavaScript("JSON.stringify({compromised:window.compromised,body:frames[0].document.body.textContent,images:[...frames[0].document.images].map(i=>({width:i.naturalWidth,src:i.src})),location:location.href})") { value, error in
            guard error == nil, let json = value as? String, let data = json.data(using: .utf8), var row = try? JSONSerialization.jsonObject(with:data) as? [String:Any] else { print("OBSERVATION FAILED: \(String(describing:error))");exit(1) }
            row["name"] = self.fixtures[self.index].name
            self.observations.append(row); self.index += 1; self.run()
        }
    }
}
let fixtures = try JSONDecoder().decode([Fixture].self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let probe = Probe(fixtures: fixtures, origin: CommandLine.arguments[2])
DispatchQueue.main.asyncAfter(deadline: .now()+20) { print("NATIVE TEST TIMEOUT");exit(2) }
probe.run();app.run()
