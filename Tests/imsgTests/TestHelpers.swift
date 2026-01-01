import Darwin
import Foundation

private actor StdoutCaptureLock {
  func withLock<T>(_ block: @Sendable () async throws -> T) async rethrows -> T {
    return try await block()
  }
}

private let stdoutCaptureLock = StdoutCaptureLock()

func captureStdout(_ block: @Sendable () async throws -> Void) async rethrows -> String {
  try await stdoutCaptureLock.withLock {
    let pipe = Pipe()
    let original = dup(STDOUT_FILENO)
    dup2(pipe.fileHandleForWriting.fileDescriptor, STDOUT_FILENO)
    defer {
      fflush(stdout)
      dup2(original, STDOUT_FILENO)
      close(original)
    }
    try await block()
    pipe.fileHandleForWriting.closeFile()
    let data = pipe.fileHandleForReading.readDataToEndOfFile()
    return String(data: data, encoding: .utf8) ?? ""
  }
}
