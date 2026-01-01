import Foundation
import Testing

@testable import imsg

@Test
func commandRouterPrintsVersionFromEnv() async throws {
  setenv("IMSG_VERSION", "9.9.9-test", 1)
  defer { unsetenv("IMSG_VERSION") }
  let router = CommandRouter()
  #expect(router.version == "9.9.9-test")
  let status = await router.run(argv: ["imsg", "--version"])
  #expect(status == 0)
}

@Test
func commandRouterPrintsHelp() async {
  let router = CommandRouter()
  let status = await router.run(argv: ["imsg", "--help"])
  #expect(status == 0)
}

@Test
func commandRouterUnknownCommand() async {
  let router = CommandRouter()
  let status = await router.run(argv: ["imsg", "nope"])
  #expect(status == 1)
}
