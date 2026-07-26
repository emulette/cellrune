"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const PACKAGE_NAME = "@cellrune/node";
// Derived from the manifest under test so a version bump cannot leave this behind.
const PACKAGE_VERSION = require("../package.json").version;
const THIRD_PARTY_LICENSES = "THIRD_PARTY_LICENSES.md";
// CellRune is dual-licensed, so every published archive carries both texts. This is the single
// definition the root and platform archive assertions below read: a boundary that ships one text
// and omits the other fails here rather than on the registry.
const LICENSE_NAMES = ["LICENSE-MIT", "LICENSE-APACHE"];
const ROOT_ARCHIVE_FILES = [
  ...LICENSE_NAMES,
  "README.md",
  THIRD_PARTY_LICENSES,
  "index.d.ts",
  "index.mjs",
  "lib/changes.js",
  "lib/errors.js",
  "lib/normalization.js",
  "lib/validation.js",
  "native.d.ts",
  "native.js",
  "package.json",
  "wrapper.js",
].sort();

function run(executable, args, options) {
  return execFileSync(executable, args, {
    encoding: "utf8",
    stdio: "pipe",
    shell:
      process.platform === "win32" &&
      executable.toLowerCase().endsWith(".cmd"),
    ...options,
  });
}

function assertPublicMetadata(manifest, directory) {
  assert.equal(manifest.homepage, "https://github.com/emulette/cellrune#readme");
  assert.deepEqual(manifest.repository, {
    type: "git",
    url: "git+https://github.com/emulette/cellrune.git",
    directory,
  });
  assert.deepEqual(manifest.bugs, {
    url: "https://github.com/emulette/cellrune/issues",
  });
  assert.deepEqual(manifest.publishConfig, {
    access: "public",
    provenance: true,
  });
}

function currentPlatformPackage() {
  const platform = process.platform;
  const architecture = process.arch;
  if (platform === "darwin" && ["arm64", "x64"].includes(architecture)) {
    return `darwin-${architecture}`;
  }
  if (platform === "win32" && ["arm64", "x64"].includes(architecture)) {
    return `win32-${architecture}-msvc`;
  }
  if (platform === "linux" && ["arm64", "x64"].includes(architecture)) {
    const report = process.report?.getReport();
    const libc = report?.header?.glibcVersionRuntime ? "gnu" : "musl";
    return `linux-${architecture}-${libc}`;
  }
  throw new Error(`unsupported package-test host: ${platform}-${architecture}`);
}

function pack(npm, sourceDirectory, packDirectory, cacheDirectory) {
  const output = run(
    npm,
    [
      "pack",
      "--pack-destination",
      packDirectory,
      "--cache",
      cacheDirectory,
      "--json",
    ],
    { cwd: sourceDirectory },
  );
  const reports = JSON.parse(output);
  assert.equal(reports.length, 1);
  const report = reports[0];
  const archive = path.join(packDirectory, report.filename);
  assert.ok(fs.existsSync(archive));
  return {
    archive,
    files: report.files.map((file) => file.path).sort(),
  };
}

function validatePlatformManifests(packageRoot) {
  const requireAllBinaries =
    process.env.CELLRUNE_REQUIRE_ALL_PLATFORM_BINARIES === "1";
  const rootManifest = JSON.parse(
    fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
  );
  assertPublicMetadata(rootManifest, "bindings/node");
  assert.ok(rootManifest.files.includes("README.md"));
  assert.ok(rootManifest.files.includes(THIRD_PARTY_LICENSES));
  assert.ok(!rootManifest.files.some((name) => name.endsWith(".node")));
  const declaredPackages = new Map(
    Object.entries(rootManifest.optionalDependencies),
  );
  const platformDirectories = fs
    .readdirSync(path.join(packageRoot, "npm"), { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name);
  assert.equal(platformDirectories.length, declaredPackages.size);
  for (const directory of platformDirectories) {
    const platformRoot = path.join(packageRoot, "npm", directory);
    const manifest = JSON.parse(
      fs.readFileSync(path.join(platformRoot, "package.json"), "utf8"),
    );
    assert.equal(manifest.version, PACKAGE_VERSION);
    assert.equal(declaredPackages.get(manifest.name), PACKAGE_VERSION);
    assert.ok(manifest.files.includes(manifest.main));
    assert.ok(manifest.files.includes("README.md"));
    assert.ok(manifest.files.includes(THIRD_PARTY_LICENSES));
    for (const licenseName of LICENSE_NAMES) {
      assert.ok(
        manifest.files.includes(licenseName),
        `${manifest.name} must declare ${licenseName}`,
      );
      assert.ok(
        fs.existsSync(path.join(platformRoot, licenseName)),
        `${manifest.name} must contain ${licenseName}`,
      );
    }
    assertPublicMetadata(manifest, `bindings/node/npm/${directory}`);
    if (requireAllBinaries) {
      assert.deepEqual(
        fs
          .readdirSync(platformRoot)
          .filter((name) => name.endsWith(".node"))
          .sort(),
        [manifest.main],
        `${manifest.name} must contain exactly its declared native binary`,
      );
      assert.ok(fs.existsSync(path.join(platformRoot, THIRD_PARTY_LICENSES)));
    }
  }
}

async function main() {
  const packageRoot = path.join(__dirname, "..");
  const temporary = fs.mkdtempSync(
    path.join(os.tmpdir(), "cellrune-node-package-"),
  );
  try {
    validatePlatformManifests(packageRoot);
    const packDirectory = path.join(temporary, "pack");
    const consumerDirectory = path.join(temporary, "consumer");
    const cacheDirectory = path.join(temporary, "npm-cache");
    fs.mkdirSync(packDirectory);
    fs.mkdirSync(consumerDirectory);

    const npm = process.platform === "win32" ? "npm.cmd" : "npm";
    const platformName = currentPlatformPackage();
    const platformStaging = path.join(temporary, "platform-package");
    fs.cpSync(path.join(packageRoot, "npm", platformName), platformStaging, {
      recursive: true,
    });
    const platformManifest = JSON.parse(
      fs.readFileSync(path.join(platformStaging, "package.json"), "utf8"),
    );
    const nativeName = platformManifest.main;
    const assembledNative = path.join(
      packageRoot,
      "npm",
      platformName,
      nativeName,
    );
    const nativeSource =
      process.env.CELLRUNE_NATIVE_BINARY ??
      (fs.existsSync(assembledNative)
        ? assembledNative
        : path.join(packageRoot, nativeName));
    fs.copyFileSync(
      nativeSource,
      path.join(platformStaging, nativeName),
    );
    const stagedNotice = path.join(platformStaging, THIRD_PARTY_LICENSES);
    const configuredNotice = process.env.CELLRUNE_THIRD_PARTY_LICENSES;
    if (configuredNotice !== undefined) {
      fs.copyFileSync(configuredNotice, stagedNotice);
    } else if (!fs.existsSync(stagedNotice)) {
      fs.copyFileSync(
        path.join(packageRoot, THIRD_PARTY_LICENSES),
        stagedNotice,
      );
    }
    const platformPack = pack(
      npm,
      platformStaging,
      packDirectory,
      cacheDirectory,
    );
    assert.deepEqual(
      platformPack.files,
      [
        ...LICENSE_NAMES,
        "README.md",
        THIRD_PARTY_LICENSES,
        nativeName,
        "package.json",
      ].sort(),
    );

    const rootPack = pack(
      npm,
      packageRoot,
      packDirectory,
      cacheDirectory,
    );
    assert.deepEqual(rootPack.files, ROOT_ARCHIVE_FILES);
    if (process.env.CELLRUNE_VERIFY_ARCHIVES === "1") {
      const python =
        process.env.CELLRUNE_PYTHON ??
        (process.platform === "win32" ? "python" : "python3");
      run(
        python,
        [
          path.join(
            packageRoot,
            "..",
            "scripts",
            "verify_release_artifacts.py",
          ),
          platformPack.archive,
          rootPack.archive,
        ],
        { cwd: packageRoot },
      );
    }

    run(
      npm,
      [
        "install",
        "--omit=optional",
        "--ignore-scripts",
        "--offline",
        "--no-audit",
        "--no-fund",
        "--cache",
        cacheDirectory,
        platformPack.archive,
      ],
      { cwd: consumerDirectory },
    );
    run(
      npm,
      [
        "install",
        "--omit=optional",
        "--ignore-scripts",
        "--offline",
        "--no-audit",
        "--no-fund",
        "--cache",
        cacheDirectory,
        rootPack.archive,
      ],
      { cwd: consumerDirectory },
    );

    run(
      process.execPath,
      [
        path.join(__dirname, "packed-runtime.cjs"),
        consumerDirectory,
        platformManifest.name,
      ],
      {
        cwd: packageRoot,
      },
    );
  } finally {
    fs.rmSync(temporary, {
      recursive: true,
      force: true,
      maxRetries: 10,
      retryDelay: 100,
    });
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack ?? error}\n`);
  process.exitCode = 1;
});
