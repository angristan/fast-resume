const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const {
  PLATFORM_PACKAGES,
  packageFor,
} = require("../lib/platform.cjs");

const packageRoot = path.resolve(__dirname, "..");
const npmRoot = path.resolve(packageRoot, "..");
const launcherPackage = require("../package.json");

test("selects each supported native package", () => {
  for (const [platformAndArch, expected] of Object.entries(PLATFORM_PACKAGES)) {
    const [platform, arch] = platformAndArch.split("-");
    assert.equal(packageFor(platform, arch), expected);
  }
});

test("rejects unsupported platforms", () => {
  assert.throws(
    () => packageFor("freebsd", "x64"),
    /does not support freebsd-x64/,
  );
});

test("keeps native package metadata in sync", () => {
  assert.deepEqual(
    Object.keys(launcherPackage.optionalDependencies).sort(),
    Object.values(PLATFORM_PACKAGES).sort(),
  );

  for (const packageAlias of Object.values(PLATFORM_PACKAGES)) {
    const metadata = JSON.parse(
      fs.readFileSync(
        path.join(npmRoot, "platforms", packageAlias, "package.json"),
      ),
    );
    const variant = packageAlias.replace(/^fast-resume-/, "");
    const variantVersion = `${launcherPackage.version}-${variant}`;

    assert.equal(metadata.name, launcherPackage.name);
    assert.equal(metadata.version, variantVersion);
    assert.equal(
      launcherPackage.optionalDependencies[packageAlias],
      `npm:${launcherPackage.name}@${variantVersion}`,
    );
  }
});
