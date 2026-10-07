//! `.bgeo.sc` container (`scf1`): framing + blosc chunks (LZ4 codec, byte-shuffle) → the inner geometry bytes (bjson).
//! Layout (inferred from the 20.5.487 shipped corpus: 11 files, all codec 1 / flags 0x21 / typesize 4, one file multi-chunk):
//!   `73 63 66 31` + 8 bytes (zero) | chunk 0 .. chunk n-1 (blosc, back to back) | index `(n-1) x u64be` (cumulative chunk
//!   offsets from byte 12) | `chunk_size u64be` | `last_chunk_nbytes u64be` | `idx_len u64be` (= 16 + 8(n-1)) | `31 66 63 73` (`1fcs`).
//! Blosc chunk: 16-byte header `version versionlz flags typesize nbytes blocksize cbytes` (u32le) + `nblocks x u32le` block starts +
//! blocks. Block = `nsplits` streams, each `i32le csize` + data (`csize == raw len` → stored raw). Then unshuffle if flag bit 0.
//! Only observed codec (1 = LZ4) is implemented; other valid ids → `Unsupported`, reserved ids (5..7) → `Syntax`.
//! Hostile input → `Err`, never panic: every offset/length checked, decompressed size capped at `MAX_OUT`.
use crate::{Detail, GeoError};

/// File magic `scf1`.
pub const MAGIC: [u8; 4] = *b"scf1";
const END_MAGIC: [u8; 4] = *b"1fcs";
/// Cap on total decompressed bytes (512 MB).
pub const MAX_OUT: usize = 512 * 1024 * 1024;
const FRAME_HEAD: usize = 12;
const CHUNK_HEAD: usize = 16;
const MAX_SPLITS: usize = 16;
const MIN_SPLIT: usize = 128;
/// LZ4 cannot expand more than ~255:1; bounds the allocation a lying `nbytes` can request.
const MAX_RATIO: usize = 260;

fn syn<T>(m: &str) -> Result<T, GeoError> {
    Err(GeoError::Syntax(format!("scf1: {m}")))
}
fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}
fn u64be(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at.checked_add(8)?)?.try_into().ok()?))
}

/// Decompress a whole `scf1` file to its inner bytes.
pub fn sc_decompress(b: &[u8]) -> Result<Vec<u8>, GeoError> {
    crate::error::check_len(b.len())?;
    if !b.starts_with(&MAGIC) || b.len() < FRAME_HEAD + 28 || !b.ends_with(&END_MAGIC) {
        return syn("bad frame magic or truncated");
    }
    let t = b.len() - 28;
    let (Some(chunk_size), Some(last), Some(idx_len)) = (u64be(b, t), u64be(b, t + 8), u64be(b, t + 16)) else {
        return syn("truncated trailer");
    };
    let idx_len = usize::try_from(idx_len).or_else(|_| syn("index length"))?;
    if idx_len < 16 || (idx_len - 16) % 8 != 0 {
        return syn("index length");
    }
    let end = b.len().checked_sub(12).and_then(|x| x.checked_sub(idx_len)).filter(|e| *e >= FRAME_HEAD);
    let Some(end) = end else { return syn("index length exceeds file") };
    let n_idx = (idx_len - 16) / 8;
    if chunk_size == 0 || chunk_size > MAX_OUT as u64 {
        return syn("chunk size");
    }
    let (mut p, mut out, mut chunks, mut last_n) = (FRAME_HEAD, Vec::<u8>::new(), 0usize, 0usize);
    while p < end {
        if chunks > 0 {
            let want = u64be(b, end + (chunks - 1) * 8).filter(|_| chunks <= n_idx);
            if want != Some((p - FRAME_HEAD) as u64) {
                return syn("chunk index mismatch");
            }
        }
        let cb = u32le(b, p + 12).map_or(0, |v| v as usize);
        let chunk = b.get(p..p.saturating_add(cb)).filter(|_| p + cb <= end);
        let Some(chunk) = chunk else { return syn("chunk overruns frame") };
        let before = out.len();
        blosc_chunk(chunk, &mut out)?;
        last_n = out.len() - before;
        if last_n as u64 > chunk_size {
            return syn("chunk exceeds declared chunk size");
        }
        p += cb;
        chunks += 1;
    }
    if p != end || chunks != n_idx + 1 || last_n as u64 != last {
        return syn("chunk count / last-chunk size disagree with trailer");
    }
    Ok(out)
}

/// Decode one blosc chunk, appending to `out` (total capped at `MAX_OUT`).
fn blosc_chunk(c: &[u8], out: &mut Vec<u8>) -> Result<(), GeoError> {
    let (Some(&version), Some(&vlz), Some(&flags), Some(&ts)) = (c.first(), c.get(1), c.get(2), c.get(3)) else {
        return syn("truncated chunk header");
    };
    let (Some(nbytes), Some(blocksize), Some(cb)) = (u32le(c, 4), u32le(c, 8), u32le(c, 12)) else {
        return syn("truncated chunk header");
    };
    let (nbytes, blocksize, cb, ts) = (nbytes as usize, blocksize as usize, cb as usize, ts as usize);
    if version != 2 || cb != c.len() || cb < CHUNK_HEAD {
        return syn("chunk version / cbytes");
    }
    if out.len().checked_add(nbytes).is_none_or(|t| t > MAX_OUT) {
        return Err(GeoError::TooLarge { len: out.len().saturating_add(nbytes), max: MAX_OUT });
    }
    if nbytes > cb.saturating_mul(MAX_RATIO) {
        return syn("nbytes implausible for cbytes");
    }
    if flags & 0x1c != 0 && flags & 0x1c != 0x2 {
        return Err(GeoError::Unsupported("scf1: bitshuffle/delta blosc flags".into()));
    }
    let (shuffle, memcpy, codec) = (flags & 1 != 0, flags & 2 != 0, flags >> 5);
    if memcpy {
        let body = &c[CHUNK_HEAD..];
        if body.len() != nbytes {
            return syn("memcpy chunk size");
        }
        out.extend_from_slice(body);
        return Ok(());
    }
    match codec {
        1 if vlz == 1 => {}
        1 => return syn("lz4 format version"),
        0 | 2 | 3 | 4 => return Err(GeoError::Unsupported(format!("scf1: blosc codec {codec} not implemented"))),
        _ => return syn("reserved blosc codec id"),
    }
    if nbytes == 0 {
        return if cb == CHUNK_HEAD { Ok(()) } else { syn("empty chunk with payload") };
    }
    if ts == 0 || blocksize == 0 || blocksize > nbytes {
        return syn("typesize / blocksize");
    }
    let nblocks = nbytes.div_ceil(blocksize);
    let leftover = nbytes % blocksize;
    let starts_end = nblocks.checked_mul(4).and_then(|x| x.checked_add(CHUNK_HEAD)).filter(|e| *e <= cb);
    let Some(starts_end) = starts_end else { return syn("block table overruns chunk") };
    let base = out.len();
    out.resize(base + nbytes, 0);
    let mut tmp = vec![0u8; blocksize];
    for i in 0..nblocks {
        let Some(start) = u32le(c, CHUNK_HEAD + i * 4).map(|v| v as usize).filter(|s| *s >= starts_end && *s < cb) else {
            return syn("block start out of range");
        };
        let is_left = leftover != 0 && i + 1 == nblocks;
        let bsize = if is_left { leftover } else { blocksize };
        let nsplits = if ts <= MAX_SPLITS && blocksize / ts >= MIN_SPLIT && !is_left { ts } else { 1 };
        if bsize % nsplits != 0 {
            return syn("block not divisible into splits");
        }
        let neblock = bsize / nsplits;
        let mut pos = start;
        for j in 0..nsplits {
            let Some(cs) = u32le(c, pos).map(|v| v as usize) else { return syn("truncated stream length") };
            pos += 4;
            let Some(src) = c.get(pos..pos.saturating_add(cs)).filter(|_| pos + cs <= cb) else {
                return syn("stream overruns chunk");
            };
            pos += cs;
            let Some(dst) = tmp.get_mut(j * neblock..(j + 1) * neblock) else { return syn("split bounds") };
            if cs == neblock {
                dst.copy_from_slice(src);
            } else {
                lz4_block(src, dst)?;
            }
        }
        let Some(dst) = out.get_mut(base + i * blocksize..base + i * blocksize + bsize) else { return syn("block bounds") };
        let Some(src) = tmp.get(..bsize) else { return syn("block bounds") };
        if shuffle && ts > 1 {
            unshuffle(ts, src, dst);
        } else {
            dst.copy_from_slice(src);
        }
    }
    Ok(())
}

/// Inverse byte-shuffle: `src` holds `ts` planes of `n = len/ts` bytes; the `len % ts` tail is stored as is.
fn unshuffle(ts: usize, src: &[u8], dst: &mut [u8]) {
    let n = src.len() / ts;
    let body = n * ts;
    for j in 0..ts {
        for i in 0..n {
            if let (Some(s), Some(d)) = (src.get(j * n + i), dst.get_mut(i * ts + j)) {
                *d = *s;
            }
        }
    }
    if let (Some(s), Some(d)) = (src.get(body..), dst.get_mut(body..)) {
        d.copy_from_slice(s);
    }
}

/// LZ4 block decompress; must fill `dst` exactly.
pub(crate) fn lz4_block(src: &[u8], dst: &mut [u8]) -> Result<(), GeoError> {
    let (mut s, mut d) = (0usize, 0usize);
    let ext = |s: &mut usize, mut n: usize| -> Result<usize, GeoError> {
        if n == 15 {
            loop {
                let Some(&x) = src.get(*s) else { return syn("lz4: truncated length") };
                *s += 1;
                n = n.checked_add(usize::from(x)).map_or_else(|| syn("lz4: length overflow"), Ok)?;
                if x != 255 {
                    break;
                }
            }
        }
        Ok(n)
    };
    while s < src.len() {
        let tok = src[s];
        s += 1;
        let lit = ext(&mut s, usize::from(tok >> 4))?;
        let (Some(l), Some(o)) = (src.get(s..s.saturating_add(lit)), dst.get_mut(d..d.saturating_add(lit))) else {
            return syn("lz4: literal overruns");
        };
        o.copy_from_slice(l);
        s += lit;
        d += lit;
        if s == src.len() {
            break;
        }
        let Some(off) = src.get(s..s + 2).filter(|_| s + 2 <= src.len()) else { return syn("lz4: truncated offset") };
        let off = usize::from(u16::from_le_bytes([off[0], off[1]]));
        s += 2;
        let m = ext(&mut s, usize::from(tok & 15))?.checked_add(4).map_or_else(|| syn("lz4: length overflow"), Ok)?;
        if off == 0 || off > d || d.checked_add(m).is_none_or(|e| e > dst.len()) {
            return syn("lz4: bad match");
        }
        for k in 0..m {
            dst[d + k] = dst[d + k - off];
        }
        d += m;
    }
    if d != dst.len() {
        return syn("lz4: output size mismatch");
    }
    Ok(())
}

/// Read a `.bgeo.sc`: decompress, then the usual sniffed reader (a nested `scf1` is refused).
pub fn read_geo_sc(bytes: &[u8]) -> Result<Detail, GeoError> {
    let inner = sc_decompress(bytes)?;
    if inner.starts_with(&MAGIC) {
        return syn("nested container");
    }
    crate::read_geo(&inner)
}
