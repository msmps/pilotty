#!/usr/bin/env node

const { existsSync } = require('node:fs');
const { join } = require('node:path');
const { spawn } = require('node:child_process');

const osMap = {
  darwin: 'darwin',
  linux: 'linux',
  win32: 'win32',
};

const archMap = {
  x64: 'x64',
  arm64: 'arm64',
};

const os = osMap[process.platform];
const arch = archMap[process.arch];

if (!os) {
  console.error(`Error: Unsupported operating system: ${process.platform}`);
  console.error('pilotty supports macOS, Linux, and Windows.');
  process.exit(1);
}

if (!arch) {
  console.error(`Error: Unsupported architecture: ${process.arch}`);
  console.error('pilotty supports x64 and arm64 architectures.');
  process.exit(1);
}

const extension = os === 'win32' ? '.exe' : '';
const binary = join(__dirname, `pilotty-${os}-${arch}${extension}`);

if (!existsSync(binary)) {
  console.error(`Error: No binary found for ${os}-${arch}`);
  console.error(`Expected: ${binary}`);
  console.error('You can build from source: https://github.com/msmps/pilotty');
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), {
  stdio: 'inherit',
  windowsHide: true,
});

child.on('error', (error) => {
  console.error(`Failed to launch pilotty: ${error.message}`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 0);
});
