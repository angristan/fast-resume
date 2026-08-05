#!/usr/bin/env node

const { spawn } = require("node:child_process");
const { resolveBinary } = require("../lib/platform.cjs");

function main() {
  let binary;
  try {
    binary = resolveBinary();
  } catch (error) {
    console.error(`fast-resume: ${error.message}`);
    process.exitCode = 1;
    return;
  }

  const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" });

  child.once("error", (error) => {
    console.error(`fast-resume: failed to start native binary: ${error.message}`);
    process.exitCode = 1;
  });

  child.once("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exitCode = code ?? 1;
  });
}

main();
