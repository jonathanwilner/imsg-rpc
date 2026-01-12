import Foundation

#if canImport(Contacts)
  import Contacts
#endif

public struct ContactMatch: Sendable, Equatable {
  public let name: String
  public let handles: [String]

  public init(name: String, handles: [String]) {
    self.name = name
    self.handles = handles
  }
}

public enum ContactLookupError: Error {
  case unauthorized
}

public enum ContactLookup {
  public static func search(query: String, limit: Int) throws -> [ContactMatch] {
    #if canImport(Contacts)
      guard ensureAccess() else { throw ContactLookupError.unauthorized }
      let store = CNContactStore()
      let predicate = CNContact.predicateForContacts(matchingName: query)
      let keys: [CNKeyDescriptor] = [
        CNContactGivenNameKey as CNKeyDescriptor,
        CNContactFamilyNameKey as CNKeyDescriptor,
        CNContactPhoneNumbersKey as CNKeyDescriptor,
        CNContactEmailAddressesKey as CNKeyDescriptor,
      ]
      let contacts = try store.unifiedContacts(matching: predicate, keysToFetch: keys)
      return contacts.prefix(max(limit, 1)).compactMap { contactMatch(from: $0) }
    #else
      return []
    #endif
  }

  public static func resolve(handles: [String]) throws -> [String: String] {
    #if canImport(Contacts)
    guard ensureAccess() else { throw ContactLookupError.unauthorized }
    let store = CNContactStore()
    var resolved: [String: String] = [:]
    var unresolvedDigits: [String: String] = [:]
    for handle in handles {
      let trimmed = handle.trimmingCharacters(in: .whitespacesAndNewlines)
      if trimmed.isEmpty { continue }
      if trimmed.contains("@") {
        let predicate = CNContact.predicateForContacts(matchingEmailAddress: trimmed)
        let contacts = try store.unifiedContacts(matching: predicate, keysToFetch: nameKeys())
        if let contact = contacts.first {
          resolved[handle] = displayName(for: contact)
        }
        continue
      }
      let candidates = phoneCandidates(from: trimmed)
      var matched = false
      for candidate in candidates {
        let phone = CNPhoneNumber(stringValue: candidate)
        let predicate = CNContact.predicateForContacts(matching: phone)
        let contacts = try store.unifiedContacts(matching: predicate, keysToFetch: nameKeys())
        if let contact = contacts.first {
          resolved[handle] = displayName(for: contact)
          matched = true
          break
        }
      }
      if !matched {
        let digits = trimmed.filter { $0.isNumber }
        if !digits.isEmpty {
          unresolvedDigits[handle] = digits
        }
      }
    }

    if !unresolvedDigits.isEmpty {
      let fetchKeys: [CNKeyDescriptor] = [
        CNContactGivenNameKey as CNKeyDescriptor,
        CNContactFamilyNameKey as CNKeyDescriptor,
        CNContactPhoneNumbersKey as CNKeyDescriptor,
      ]
      var phoneIndex: [String: String] = [:]
      let request = CNContactFetchRequest(keysToFetch: fetchKeys)
      try store.enumerateContacts(with: request) { contact, _ in
        let name = displayName(for: contact)
        for number in contact.phoneNumbers {
          let raw = number.value.stringValue
          let digits = raw.filter { $0.isNumber }
          if digits.isEmpty { continue }
          phoneIndex[digits] = name
          if digits.count > 10 {
            phoneIndex[String(digits.suffix(10))] = name
          }
        }
      }
      for (handle, digits) in unresolvedDigits {
        if let name = phoneIndex[digits] ?? phoneIndex[String(digits.suffix(10))] {
          resolved[handle] = name
        }
      }
    }
    return resolved
    #else
    return [:]
    #endif
  }

  #if canImport(Contacts)
    private static func ensureAccess() -> Bool {
      let status = CNContactStore.authorizationStatus(for: .contacts)
      switch status {
      case .authorized:
        return true
      case .notDetermined:
        let store = CNContactStore()
        let semaphore = DispatchSemaphore(value: 0)
        var granted = false
        store.requestAccess(for: .contacts) { ok, _ in
          granted = ok
          semaphore.signal()
        }
        semaphore.wait()
        return granted
      default:
        return false
      }
    }

    private static func nameKeys() -> [CNKeyDescriptor] {
      [
        CNContactGivenNameKey as CNKeyDescriptor,
        CNContactFamilyNameKey as CNKeyDescriptor,
      ]
    }

    private static func displayName(for contact: CNContact) -> String {
      let name = "\(contact.givenName) \(contact.familyName)".trimmingCharacters(in: .whitespaces)
      return name.isEmpty ? "Unknown" : name
    }

  private static func contactMatch(from contact: CNContact) -> ContactMatch? {
    let name = displayName(for: contact)
    let phones = contact.phoneNumbers.map { $0.value.stringValue }
    let emails = contact.emailAddresses.map { String($0.value) }
    let handles = (phones + emails).filter { !$0.isEmpty }
    guard !handles.isEmpty else { return nil }
    return ContactMatch(name: name, handles: handles)
  }

  private static func phoneCandidates(from raw: String) -> [String] {
    var candidates: [String] = []
    let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
    if !trimmed.isEmpty {
      candidates.append(trimmed)
    }
    let digits = trimmed.filter { $0.isNumber }
    if !digits.isEmpty {
      if !candidates.contains(digits) {
        candidates.append(digits)
      }
      if digits.count > 10 {
        let last10 = String(digits.suffix(10))
        if !candidates.contains(last10) {
          candidates.append(last10)
        }
        let plus1 = "+1" + last10
        if !candidates.contains(plus1) {
          candidates.append(plus1)
        }
      } else if digits.count == 10 {
        let plus1 = "+1" + digits
        if !candidates.contains(plus1) {
          candidates.append(plus1)
        }
      } else if digits.count == 11, digits.hasPrefix("1") {
        let plus1 = "+" + digits
        if !candidates.contains(plus1) {
          candidates.append(plus1)
        }
      }
    }
    return candidates
  }
  #endif
}
