import XCTest
@testable import AgentRouter

/// Where a router is: the routers phones saved before https (no scheme, plain HTTP), a
/// hosted router added by its https URL, and what the add form makes of what is typed.
final class RouterAddressTests: XCTestCase {
    /// A routers.json written before https: the four keys every saved router has, and no
    /// `scheme`. It decodes, and points exactly where it always did.
    func testARouterSavedBeforeHttpsIsStillPlainHttp() throws {
        let saved = #"[{"name":"Mac","id":"8C1F2B4E-0D3A-4E8B-9A6C-2F5D7E9B1A30","host":"10.0.0.5","port":8787},{"name":"Spare","id":"1E7A9C3D-5B2F-4A6E-8D0C-7F3B1A9E5C42","host":"agent-router.example.com","port":8799}]"#
        let routers = try JSONDecoder().decode([Router].self, from: Data(saved.utf8))
        XCTAssertEqual(routers.map(\.name), ["Mac", "Spare"])
        let mac = routers[0]
        XCTAssertEqual(mac.id, UUID(uuidString: "8C1F2B4E-0D3A-4E8B-9A6C-2F5D7E9B1A30"))
        XCTAssertNil(mac.scheme, "nothing was saved, and nothing is made up")
        XCTAssertEqual(mac.baseURL?.scheme, "http")
        XCTAssertEqual(mac.baseURL?.absoluteString, "http://10.0.0.5:8787")
        XCTAssertEqual(mac.address, "10.0.0.5:8787")
        for r in routers {
            // The URL the app built before https, verbatim.
            XCTAssertEqual(r.baseURL, URL(string: "http://\(r.host.trimmingCharacters(in: .whitespaces)):\(r.port)"), r.name)
            XCTAssertEqual(r.address, "\(r.host):\(r.port)", r.name)
        }
        XCTAssertEqual(routers[1].baseURL?.absoluteString, "http://agent-router.example.com:8799")

        // Saved again, the list is written as it was read: no `scheme` key appears.
        let rewritten = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(routers)) as? [[String: Any]])
        XCTAssertEqual(rewritten.map { Set($0.keys) }, [["id", "name", "host", "port"], ["id", "name", "host", "port"]])
        XCTAssertEqual(try JSONDecoder().decode([Router].self, from: JSONEncoder().encode(routers)), routers)
    }

    /// A hosted router, behind its ingress: https by name, 443 unless it says otherwise.
    func testAHostedRouterIsReachedOverHttps() throws {
        let hosted = Router(name: "Hosted", host: "agent-router.example.com", port: 443, scheme: .https)
        XCTAssertEqual(hosted.baseURL?.absoluteString, "https://agent-router.example.com")
        XCTAssertEqual(hosted.address, "https://agent-router.example.com")
        // The client asks for paths on it, exactly as it does on the LAN.
        XCTAssertEqual(URL(string: "/_router/health", relativeTo: hosted.baseURL)?.absoluteURL.absoluteString,
                       "https://agent-router.example.com/_router/health")

        let otherPort = Router(name: "Hosted", host: "agent-router.example.com", port: 8443, scheme: .https)
        XCTAssertEqual(otherPort.baseURL?.absoluteString, "https://agent-router.example.com:8443")
        XCTAssertEqual(otherPort.address, "https://agent-router.example.com:8443")
        XCTAssertEqual(URL(string: "/api/live/sessions", relativeTo: otherPort.baseURL)?.absoluteURL.absoluteString,
                       "https://agent-router.example.com:8443/api/live/sessions")

        // Saved and read back, it stays https.
        let back = try JSONDecoder().decode(Router.self, from: JSONEncoder().encode(otherPort))
        XCTAssertEqual(back, otherPort)
        XCTAssertEqual(back.scheme, .https)

        // A router the form saves as plain HTTP is the same address a saved one always was.
        let lan = Router(name: "Mac", host: "10.0.0.5", port: 8787, scheme: .http)
        XCTAssertEqual(lan.baseURL?.absoluteString, "http://10.0.0.5:8787")
        XCTAssertEqual(lan.address, "10.0.0.5:8787")
    }

    /// The form takes a whole URL, which decides its own port, or a bare host with the
    /// port beside it, which is plain HTTP as before.
    func testWhatTheFormMakesOfAnAddress() {
        func entry(_ host: String, _ port: String = "8787") -> Router.Entry { Router.entry(host: host, port: port) }

        XCTAssertEqual(entry("10.0.0.5"), .valid(scheme: .http, host: "10.0.0.5", port: 8787))
        XCTAssertEqual(entry("  10.0.0.5 \n", " 8799 "), .valid(scheme: .http, host: "10.0.0.5", port: 8799))
        XCTAssertEqual(entry("agent-router.example.com", "443"), .valid(scheme: .http, host: "agent-router.example.com", port: 443),
                       "no scheme typed is plain HTTP, whatever the port")

        // The port field is ignored once a scheme is typed: the URL names its own, or
        // means the scheme's.
        XCTAssertEqual(entry("https://agent-router.example.com"), .valid(scheme: .https, host: "agent-router.example.com", port: 443))
        XCTAssertEqual(entry("https://agent-router.example.com/", ""), .valid(scheme: .https, host: "agent-router.example.com", port: 443))
        XCTAssertEqual(entry("HTTPS://agent-router.example.com"), .valid(scheme: .https, host: "agent-router.example.com", port: 443))
        XCTAssertEqual(entry("https://agent-router.example.com:8443"), .valid(scheme: .https, host: "agent-router.example.com", port: 8443))
        XCTAssertEqual(entry("http://10.0.0.5:8787", "1"), .valid(scheme: .http, host: "10.0.0.5", port: 8787))
        XCTAssertEqual(entry("http://10.0.0.5"), .valid(scheme: .http, host: "10.0.0.5", port: 80))

        XCTAssertEqual(entry(""), .empty)
        XCTAssertEqual(entry(" \n"), .empty)

        let refused: [(String, String)] = [
            ("10.0.0.5", ""), ("10.0.0.5", "0"), ("10.0.0.5", "65536"), ("10.0.0.5", "http"),
            ("10.0.0.5:8787", "8787"), ("10.0.0.5/dashboard", "8787"), ("10.0.0.5?x", "8787"),
            ("app@10.0.0.5", "8787"), ("my router", "8787"),
            ("ftp://agent-router.example.com", ""), ("https://", ""), ("https://agent router.example.com", ""),
            ("https://agent-router.example.com/api", ""), ("https://agent-router.example.com/?x=1", ""),
            ("https://agent-router.example.com/#settings", ""), ("https://app:secret@agent-router.example.com", ""),
            ("https://agent-router.example.com:0", ""), ("https://agent-router.example.com:70000", ""),
        ]
        for (host, port) in refused {
            if case .invalid = entry(host, port) { continue }
            XCTFail("\(host) port \(port) should be refused, got \(entry(host, port))")
        }
    }
}
