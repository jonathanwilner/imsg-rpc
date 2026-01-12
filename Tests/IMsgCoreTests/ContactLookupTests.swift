import Foundation
import Testing

@testable import IMsgCore

@Test
func contactLookupFindsJonathanWilner() throws {
  guard ProcessInfo.processInfo.environment["IMSG_CONTACT_TEST"] == "1" else {
    return
  }
#if canImport(Contacts)
  let targetHandle = "+13103590308"
  let matches = try ContactLookup.search(query: "Jonathan Wilner", limit: 5)
  let hasHandle = matches.contains { match in
    match.handles.contains { normalizeHandle($0) == normalizeHandle(targetHandle) }
  }
  #expect(hasHandle == true)

  let resolved = try ContactLookup.resolve(handles: [targetHandle])
  let name = resolved[targetHandle] ?? ""
  let lower = name.lowercased()
  #expect(lower.contains("jonathan"))
  #expect(lower.contains("wilner"))
#else
  return
#endif
}

private func normalizeHandle(_ handle: String) -> String {
  let digits = handle.filter { $0.isNumber }
  if digits.count > 10 {
    return String(digits.suffix(10))
  }
  return digits
}
