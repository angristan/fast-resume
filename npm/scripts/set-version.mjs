import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error("Usage: node npm/scripts/set-version.mjs <semver>");
  process.exit(1);
}

const npmRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = path.resolve(npmRoot, "..");
const launcherPath = path.join(npmRoot, "fast-resume", "package.json");
const platformsRoot = path.join(npmRoot, "platforms");

// The first top-level `version = "..."` line is the package version in both
// Cargo.toml ([package]) and pyproject.toml ([project]); dependency versions
// only appear in inline tables.
function setTomlVersion(file) {
  const source = fs.readFileSync(file, "utf8");
  if (!/^version = ".*"$/m.test(source)) {
    throw new Error(`No version field found in ${file}`);
  }
  fs.writeFileSync(
    file,
    source.replace(/^version = ".*"$/m, `version = "${version}"`),
  );
}

setTomlVersion(path.join(repoRoot, "Cargo.toml"));
setTomlVersion(path.join(repoRoot, "pyproject.toml"));

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

const platformPackages = fs
  .readdirSync(platformsRoot, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => ({
    alias: entry.name,
    file: path.join(platformsRoot, entry.name, "package.json"),
    variant: entry.name.replace(/^fast-resume-/, ""),
  }))
  .sort((left, right) => left.alias.localeCompare(right.alias));

const launcher = readJson(launcherPath);
const platformAliases = platformPackages.map(({ alias }) => alias);
const dependencyNames = Object.keys(launcher.optionalDependencies).sort();

if (JSON.stringify(platformAliases) !== JSON.stringify(dependencyNames)) {
  throw new Error("Native package directories and optionalDependencies differ");
}

launcher.version = version;
for (const { alias, variant } of platformPackages) {
  launcher.optionalDependencies[alias] = `npm:fast-resume@${version}-${variant}`;
}
writeJson(launcherPath, launcher);

for (const { file, variant } of platformPackages) {
  const metadata = readJson(file);
  metadata.name = "fast-resume";
  metadata.version = `${version}-${variant}`;
  writeJson(file, metadata);
}
