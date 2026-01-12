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
  let targetDigits = normalizeHandle(targetHandle)
  let matches = try ContactLookup.search(query: "Jonathan Wilner", limit: 5)
  let matchedHandle = matches
    .flatMap { $0.handles }
    .first { normalizeHandle($0) == targetDigits }
  #expect(matchedHandle != nil)

  let handle = matchedHandle ?? targetHandle
  let resolved = try ContactLookup.resolve(handles: [handle])
  let name = resolved[handle] ?? ""
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
