// swift-tools-version: 6.0
// demi-ios-sim, the iOS helper demi-mobile carries (`mobile.md` § iOS): it
// links Meta's FBSimulatorControl (MIT) from idb's v1.7.6 release, each
// library pinned by its URL and sha256. `bun xtask native build` builds it,
// with the guest bridge from the same release's source.
import PackageDescription

let release = "https://github.com/facebook/idb/releases/download/v1.7.6"

/// idb's libraries the helper links, each with its archive's sha256.
let libraries: [(String, String)] = [
  ("AccessibilityPlatformTranslationStub", "76bb2fe0fe52d9397ea0a99c78f0b94093382c92dc612a0e1ba09a9eea546599"),
  ("CoreSimulatorStub", "fdff3d257f374009ca011a8d82ac20cfa59ca5ab33b6328eb038279b353325a7"),
  ("FBControlCore", "29c1f483f528bace1d4179671413a8f62257456baf7c1981688b6e541772618d"),
  ("FBControlCoreObjC", "d597f5c66217b6156abef93f3f4a3aadb9e25cd62c88354b76b30f2c12cce2ca"),
  ("FBSimulatorAX", "7867171fd6812f43867879861ac6dba488b84c80e0fb6893a8b77911e08d6757"),
  ("FBSimulatorBridgeProtocol", "31a40d97d72c55cc0a4bd3baf180481cb8aa7c6383c94202dd72a686f0fd287a"),
  ("FBSimulatorControl", "1e1a22db29983dbc389e62b086946cac94b93b1cdd01c49dc34eed1329aa3eb7"),
  ("FBSimulatorServices", "58148470c97954f0ed074403c20b2064ec3eb69ff85fc790d1ec554b861660fb"),
  ("FBSimulatorVideo", "a87e22d8ee22494da11ccaf84f1e9f0bf2b3a37d143be4584a239c9caaf0b29e"),
  ("FBVideoCore", "1e281b14ed3e2c83e052a71edf138d9abde05c1f73c87e1d7484357f6af39c8c"),
  ("IDBPrivateHeaders", "f0ef25193330838bb049bf1af2f2788bc4750f4589076337c73f3192a4684876"),
  ("SimulatorIPC", "819008698d296584da8acdd87e516d7d94d58d7d496b6ac5740abbeb7dc8ccea"),
]

let package = Package(
  name: "DemiIosSim",
  // idb's libraries are built for macOS 15 on Apple silicon.
  platforms: [.macOS(.v15)],
  products: [.executable(name: "demi-ios-sim", targets: ["DemiIosSim"])],
  targets: libraries.map { name, checksum in
    .binaryTarget(name: name, url: "\(release)/\(name).xcframework.zip", checksum: checksum)
  } + [
    // The device's frame as DeviceKit draws it, apart from the simulator
    // so its geometry is tested on its own (`swift test`).
    .target(
      name: "DemiChrome",
      path: "Sources/DemiChrome",
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .testTarget(
      name: "DemiChromeTests",
      dependencies: ["DemiChrome"],
      path: "Tests/DemiChromeTests",
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .executableTarget(
      name: "DemiIosSim",
      dependencies: libraries.map { .byName(name: $0.0) } + ["DemiChrome"],
      path: "Sources/DemiIosSim",
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
  ]
)
