#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

function fail(message) {
  process.stderr.write(`perfect-doc: ${message}\n`);
  process.exitCode = 2;
}

function linuxLibc() {
  if (!process.report || typeof process.report.getReport !== 'function') {
    return null;
  }

  try {
    const report = process.report.getReport();
    return report.header.glibcVersionRuntime ? 'glibc' : 'musl';
  } catch {
    return null;
  }
}

function platformPackage() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === 'darwin' && arch === 'arm64') return ['perfect-doc-darwin-arm64', null];
  if (platform === 'darwin' && arch === 'x64') return ['perfect-doc-darwin-x64', null];
  if (platform === 'win32' && arch === 'x64') return ['perfect-doc-win32-x64', null];
  if (platform === 'linux' && arch === 'x64') {
    const libc = linuxLibc();
    if (libc) return [`perfect-doc-linux-x64-${libc === 'glibc' ? 'gnu' : 'musl'}`, libc];
  }

  return [null, null];
}

const [packageName, libc] = platformPackage();
if (!packageName) {
  fail(`no native binary package is available for ${process.platform}/${process.arch}`);
} else {
  let manifestPath;
  let manifest;
  try {
    manifestPath = require.resolve(`${packageName}/package.json`, { paths: [__dirname] });
    manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  } catch {
    fail(`native binary package ${packageName} is missing or incomplete`);
  }

  if (manifest) {
    const expectedOs = process.platform;
    const expectedCpu = process.arch;
    const osMatches = Array.isArray(manifest.os) && manifest.os.includes(expectedOs);
    const cpuMatches = Array.isArray(manifest.cpu) && manifest.cpu.includes(expectedCpu);
    const libcMatches = libc === null || manifest.libc === libc;
    const binaryName = manifest.bin && manifest.bin['perfect-doc'];

    if (!osMatches || !cpuMatches || !libcMatches || typeof binaryName !== 'string') {
      fail(`native binary package ${packageName} does not match this platform or is incomplete`);
    } else {
      const binaryPath = path.resolve(path.dirname(manifestPath), binaryName);
      if (!fs.existsSync(binaryPath)) {
        fail(`native binary is missing from package ${packageName}`);
      } else {
        const child = spawnSync(binaryPath, process.argv.slice(2), {
          cwd: process.cwd(),
          stdio: 'inherit',
          windowsHide: true,
        });

        if (child.error) {
          fail(`could not start native binary: ${child.error.message}`);
        } else if (child.signal) {
          try {
            process.kill(process.pid, child.signal);
          } catch {
            fail(`native binary stopped with signal ${child.signal}`);
          }
        } else {
          process.exitCode = child.status === null ? 2 : child.status;
        }
      }
    }
  }
}
