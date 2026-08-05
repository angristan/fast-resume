const PLATFORM_PACKAGES = Object.freeze({
  "darwin-arm64": "fast-resume-darwin-arm64",
  "darwin-x64": "fast-resume-darwin-x64",
  "linux-arm64": "fast-resume-linux-arm64",
  "linux-x64": "fast-resume-linux-x64",
  "win32-x64": "fast-resume-win32-x64",
});

function packageFor(platform = process.platform, arch = process.arch) {
  const packageName = PLATFORM_PACKAGES[`${platform}-${arch}`];
  if (packageName) {
    return packageName;
  }

  const supported = Object.keys(PLATFORM_PACKAGES).join(", ");
  throw new Error(
    `fast-resume does not support ${platform}-${arch}. Supported platforms: ${supported}`,
  );
}

function resolveBinary(platform = process.platform, arch = process.arch) {
  const packageName = packageFor(platform, arch);
  const executable = platform === "win32" ? "fr.exe" : "fr";

  try {
    return require.resolve(`${packageName}/bin/${executable}`);
  } catch (cause) {
    throw new Error(
      `The native package ${packageName} is missing. Reinstall fast-resume without omitting optional dependencies.`,
      { cause },
    );
  }
}

module.exports = { PLATFORM_PACKAGES, packageFor, resolveBinary };
