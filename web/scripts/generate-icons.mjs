/**
 * Generates the PWA icons and the public site's favicons from the mistwood
 * palette.
 *
 * Run with `pnpm run icons`. Output is committed, so this only needs re-running
 * when the theme changes — which is the point of having it in the repo rather
 * than a folder of binaries nobody can regenerate.
 *
 * No image dependency: the scene is a pure function of (x, y), sampled 4×4 per
 * output pixel for anti-aliasing, then encoded as PNG with node:zlib. A canvas
 * or rasteriser library would be a lot of install for a handful of flat images.
 */

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

/** PWA icons, served off write.youwin.dev. */
const OUT_DIR = join(HERE, "..", "public", "icons");

/**
 * Favicons for the public site. These sit in the repo's `static/`, which CI
 * copies over the public build output — they are not Vite inputs, so they do
 * not belong under `web/public/`.
 */
const STATIC_DIR = join(HERE, "..", "..", "static");

// sRGB conversions of the theme's OKLCH tokens (web/src/theme.css). Hardcoded
// rather than converted at runtime — these are the same four colours the CSS
// resolves to, and a colour-space implementation here would be a second source
// of truth for the palette.
const BASE_100 = [9, 18, 13]; // oklch(17% 0.016 162) — forest floor
const PRIMARY = [109, 191, 145]; // oklch(74% 0.105 158) — lichen glow
const MIST = [222, 233, 226]; // oklch(91% 0.014 152)

const SUPERSAMPLE = 4;

const lerp = (a, b, t) => a + (b - a) * t;
const mix = (a, b, t) => [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)];
const clamp01 = (v) => Math.min(1, Math.max(0, v));

/**
 * Three conifers receding into mist, over a lifted horizon.
 *
 * Coordinates are normalised to [0,1] so the same scene renders at any size.
 * `scale` shrinks the artwork toward the centre for the maskable variant, whose
 * corners get cropped to a circle by the launcher.
 */
function scene(u, v, scale) {
  // Pull toward centre for maskable padding.
  const x = (u - 0.5) / scale + 0.5;
  const y = (v - 0.5) / scale + 0.5;

  let colour = BASE_100;

  // Mist glow from above — the same radial lift the site's body carries, so the
  // icon and the page read as one thing.
  const glow = clamp01(1 - Math.hypot((x - 0.5) * 1.5, (y + 0.05) * 1.15));
  colour = mix(colour, mix(BASE_100, MIST, 0.18), glow * glow);

  // Trees, far to near. Each is a triangle; the farther ones sit paler and
  // higher, which is what reads as depth rather than as three identical shapes.
  // Bases run off the bottom edge so the trees are rooted in something instead
  // of floating over empty ground.
  const trees = [
    { cx: 0.28, base: 1.02, height: 0.66, halfWidth: 0.15, tint: 0.28 },
    { cx: 0.74, base: 1.02, height: 0.56, halfWidth: 0.13, tint: 0.42 },
    { cx: 0.5, base: 1.06, height: 0.82, halfWidth: 0.21, tint: 1 },
  ];

  for (const tree of trees) {
    const top = tree.base - tree.height;
    if (y < top || y > tree.base) continue;

    // Widens linearly from apex to base.
    const progress = (y - top) / tree.height;
    const spread = tree.halfWidth * progress;
    if (Math.abs(x - tree.cx) > spread) continue;

    // Distant trees are washed toward the background, not merely darkened.
    const foliage = mix(mix(BASE_100, PRIMARY, 0.35), PRIMARY, tree.tint);
    // Slightly deeper toward the base: light falls from above, and a flat fill
    // reads as a paper cut-out.
    colour = mix(foliage, mix(foliage, BASE_100, 0.35), progress * 0.55);
  }

  // A horizontal mist band was tried here and removed: at 192px it reads as a
  // rendering seam rather than as weather, and it was necessarily strongest at
  // the frame edges — where the trees are not — which is backwards. The glow
  // above and the gradient within the foliage carry the atmosphere on their own.

  return colour;
}

function render(size, scale) {
  const pixels = Buffer.alloc(size * size * 4);

  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let r = 0;
      let g = 0;
      let b = 0;

      for (let sy = 0; sy < SUPERSAMPLE; sy++) {
        for (let sx = 0; sx < SUPERSAMPLE; sx++) {
          const u = (px + (sx + 0.5) / SUPERSAMPLE) / size;
          const v = (py + (sy + 0.5) / SUPERSAMPLE) / size;
          const [cr, cg, cb] = scene(u, v, scale);
          r += cr;
          g += cg;
          b += cb;
        }
      }

      const samples = SUPERSAMPLE * SUPERSAMPLE;
      const offset = (py * size + px) * 4;
      pixels[offset] = Math.round(r / samples);
      pixels[offset + 1] = Math.round(g / samples);
      pixels[offset + 2] = Math.round(b / samples);
      pixels[offset + 3] = 255;
    }
  }

  return pixels;
}

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);

  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));

  return Buffer.concat([length, body, crc]);
}

function encodePng(size, pixels) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // colour type: RGBA
  // compression, filter, interlace all 0

  // Each scanline is prefixed with its filter byte. Filter 0 (none) throughout:
  // these are flat gradients, so a smarter filter would buy very little and the
  // files are a few KB either way.
  const raw = Buffer.alloc(size * (size * 4 + 1));
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    pixels.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/**
 * An .ico wrapping PNG payloads.
 *
 * The BMP form an .ico traditionally carries wants a bottom-up pixel array and a
 * separate 1-bit AND mask for transparency. The PNG form needs neither, and
 * every browser has understood it since IE11 — so with `encodePng` already
 * written this is a 6-byte header plus one 16-byte directory entry per size.
 */
function encodeIco(images) {
  const HEADER = 6;
  const ENTRY = 16;

  const header = Buffer.alloc(HEADER);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // 1 = icon, 2 = cursor
  header.writeUInt16LE(images.length, 4);

  const directory = Buffer.alloc(ENTRY * images.length);
  let offset = HEADER + directory.length;

  images.forEach(({ size, png }, index) => {
    const at = index * ENTRY;
    // Width and height are one byte each, so 256 is encoded as 0. Nothing here
    // is that big, but the field is the reason no .ico can exceed it.
    directory[at] = size === 256 ? 0 : size;
    directory[at + 1] = size === 256 ? 0 : size;
    directory[at + 2] = 0; // palette size; 0 for truecolour
    directory[at + 3] = 0; // reserved
    directory.writeUInt16LE(1, at + 4); // colour planes
    directory.writeUInt16LE(32, at + 6); // bits per pixel
    directory.writeUInt32LE(png.length, at + 8);
    directory.writeUInt32LE(offset, at + 12);
    offset += png.length;
  });

  return Buffer.concat([header, directory, ...images.map((image) => image.png)]);
}

const ICONS = [
  { name: "icon-192.png", size: 192, scale: 1 },
  { name: "icon-512.png", size: 512, scale: 1 },
  // Maskable icons get cropped to a circle inscribed in the middle 80%, so the
  // artwork shrinks to survive it while the background still bleeds to the edge.
  { name: "icon-maskable-512.png", size: 512, scale: 0.7 },
  { name: "apple-touch-icon.png", size: 180, scale: 1 },
];

/**
 * A favicon is read at 16px in a tab strip. The PWA framing leaves air around
 * the trees, and at 16px that air is most of the image — so the favicon crops
 * in until the foliage fills the frame. Same scene and same palette as the app
 * icon; just standing closer to it.
 */
const FAVICON_SCALE = 1.5;

/**
 * 16 and 32 are what browsers ask for; 48 is what Windows uses for a pinned
 * site, and it is 2 kB.
 */
const FAVICON_SIZES = [16, 32, 48];

mkdirSync(OUT_DIR, { recursive: true });

for (const { name, size, scale } of ICONS) {
  const png = encodePng(size, render(size, scale));
  writeFileSync(join(OUT_DIR, name), png);
  console.log(`${name}  ${size}×${size}  ${(png.length / 1024).toFixed(1)} kB`);
}

mkdirSync(STATIC_DIR, { recursive: true });

const favicons = FAVICON_SIZES.map((size) => ({
  size,
  png: encodePng(size, render(size, FAVICON_SCALE)),
}));

const ico = encodeIco(favicons);
writeFileSync(join(STATIC_DIR, "favicon.ico"), ico);
console.log(
  `favicon.ico  ${FAVICON_SIZES.join("/")}  ${(ico.length / 1024).toFixed(1)} kB`,
);

// Linked alongside the .ico so a browser that would rather have a PNG does not
// have to unpack one out of an icon container to get it.
for (const { size, png } of favicons) {
  if (size === 48) continue; // .ico only — nothing in the HTML asks for 48.
  const name = `favicon-${size}x${size}.png`;
  writeFileSync(join(STATIC_DIR, name), png);
  console.log(`${name}  ${size}×${size}  ${(png.length / 1024).toFixed(1)} kB`);
}
