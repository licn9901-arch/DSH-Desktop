import fs from 'node:fs';
import path from 'node:path';

import { projectRoot, sha256 } from './build-support.mjs';

const iconRoot = path.join(projectRoot, 'src-tauri', 'icons');
const expected = {
  'whale.svg': '9e983b4f649c25c6ca0623a50be1a6e705fd8f49f756638b980fa13b40575ab6',
  'icon.png': '0518574cca49b23c50f78f15075ce07080f70b8b52a3be575c0fd9f2803771de',
  'icon.ico': '1e01fb7b71b7cfc306e1704500722a4f3641c816e4497ca2a8f980cd2981ed71',
};
for (const [file, hash] of Object.entries(expected)) {
  const filePath = path.join(iconRoot, file);
  if (!fs.existsSync(filePath)) throw new Error(`Icon asset is missing: ${filePath}`);
  if (sha256(filePath) !== hash) throw new Error(`Icon asset hash mismatch: ${file}`);
}

const png = fs.readFileSync(path.join(iconRoot, 'icon.png'));
if (png.readUInt32BE(0) !== 0x89504e47 || png.readUInt32BE(16) !== 512 || png.readUInt32BE(20) !== 512) throw new Error('icon.png must be a 512x512 PNG.');

const ico = fs.readFileSync(path.join(iconRoot, 'icon.ico'));
if (ico.readUInt16LE(0) !== 0 || ico.readUInt16LE(2) !== 1) throw new Error('icon.ico has an invalid ICO header.');
const count = ico.readUInt16LE(4);
const sizes = [];
for (let index = 0; index < count; index += 1) {
  const offset = 6 + index * 16;
  const width = ico[offset] || 256;
  const height = ico[offset + 1] || 256;
  if (width !== height) throw new Error(`icon.ico contains a non-square frame: ${width}x${height}`);
  sizes.push(width);
}
sizes.sort((a, b) => a - b);
if ([...new Set(sizes)].join(',') !== '16,24,32,48,64,128,256') throw new Error(`icon.ico sizes are invalid: ${sizes.join(',')}`);
console.log('Icon assets are valid and unchanged.');
