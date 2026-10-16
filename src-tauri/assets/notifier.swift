// Supacast notification helper.
//
// Runs inside SupacastNotifier.app (bare `applet` executable) and posts a
// notification via UNUserNotificationCenter — the real API, with proper
// authorization handling — so banners carry the app bundle's rocket icon.
// AppleScript `display notification` applets silently no-op on current
// macOS; this is the reliable route.
//
// Usage: applet "title" "body"
// When SUPACAST_AUTH_STAMP (file path) is set in the environment and the
// authorization is granted, the file is touched so the caller knows the
// permission prompt was approved and future posts can go direct.
//
// Exit codes: 0 posted, 2 authorization denied, 3 post failed, 1 bad args.

import Foundation
import UserNotifications
import AppKit

let args = CommandLine.arguments

// Present ourselves as a real GUI application. Without this the system
// treats the process as a headless agent and silently denies notification
// authorization (UNErrorDomain Code=1, no prompt ever shown).
let app = NSApplication.shared
app.setActivationPolicy(.regular)
app.activate(ignoringOtherApps: true)

// --status: print the current authorization state and exit.
// 0 notDetermined, 1 denied, 2 authorized, 3 provisional.
if args.count == 2, args[1] == "--status" {
    let sem = DispatchSemaphore(value: 0)
    UNUserNotificationCenter.current().getNotificationSettings { s in
        FileHandle.standardError.write("authorizationStatus=\(s.authorizationStatus.rawValue)\n".data(using: .utf8)!)
        exit(s.authorizationStatus.rawValue == 2 ? 0 : 2)
    }
    sem.wait()
}

guard args.count >= 3 else {
    FileHandle.standardError.write("usage: applet \"title\" \"body\"\n".data(using: .utf8)!)
    exit(1)
}

let center = UNUserNotificationCenter.current()

var exitCode: Int32 = 0
let done = DispatchSemaphore(value: 0)

center.requestAuthorization(options: [.alert, .sound]) { granted, error in
    if let error = error {
        FileHandle.standardError.write("auth error: \(error)\n".data(using: .utf8)!)
    }
    guard granted else {
        FileHandle.standardError.write("notification authorization denied\n".data(using: .utf8)!)
        exitCode = 2
        done.signal()
        return
    }

    // Record the approval so the caller can skip the bootstrap flow.
    if let stamp = ProcessInfo.processInfo.environment["SUPACAST_AUTH_STAMP"] {
        try? Data("ok\n".data(using: .utf8)!).write(to: URL(fileURLWithPath: stamp))
    }

    let content = UNMutableNotificationContent()
    content.title = args[1]
    content.body = args[2]
    content.sound = .default

    let request = UNNotificationRequest(
        identifier: UUID().uuidString,
        content: content,
        trigger: nil
    )
    center.add(request) { error in
        if let error = error {
            FileHandle.standardError.write("post error: \(error)\n".data(using: .utf8)!)
            exitCode = 3
        }
        done.signal()
    }
}

// Pump the main run loop while waiting: gives NSApplication a chance to
// process activation and lets the system present the permission prompt.
while true {
    if done.wait(timeout: .now() + 0.2) == .success { break }
    app.updateWindows()
    RunLoop.main.run(mode: .default, before: Date(timeIntervalSinceNow: 0.2))
}
exit(exitCode)
