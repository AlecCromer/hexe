//! `scf1` / blosc / LZ4 reader tests: shipped sphere, synthetic frames, hostile-input regressions (never panic).
use crate::blosc::{MAGIC, lz4_block, sc_decompress};
use crate::*;
use std::path::PathBuf;

fn sphere() -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/houdini-ship/sphere-sc/sphere.bgeo.sc"),
    )
    .expect("sphere.bgeo.sc")
}
/// Frame of memcpy-flag blosc chunks (`flags 0x02`), correct index + trailer.
fn frame(chunks: &[&[u8]]) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([0u8; 8]);
    let mut offs = Vec::new();
    for c in chunks {
        offs.push((f.len() - 12) as u64);
        f.extend([2, 1, 2, 1]);
        f.extend((c.len() as u32).to_le_bytes());
        f.extend((c.len().max(1) as u32).to_le_bytes());
        f.extend((c.len() as u32 + 16).to_le_bytes());
        f.extend(*c);
    }
    for o in offs.iter().skip(1) {
        f.extend(o.to_be_bytes());
    }
    f.extend(1_048_576u64.to_be_bytes());
    f.extend((chunks.last().map_or(0, |c| c.len()) as u64).to_be_bytes());
    f.extend((16 + 8 * (chunks.len() as u64 - 1)).to_be_bytes());
    f.extend(b"1fcs");
    f
}

#[test]
fn sphere_decompresses_to_bjson_with_oracle_size() {
    let d = sc_decompress(&sphere()).unwrap();
    assert_eq!(d.len(), 23042); // blosc header nbytes, python-blosc agrees
    assert!(d.starts_with(&[0x7f, b'N', b'S', b'J', b'b']));
    let g = read_geo(&sphere()).unwrap();
    assert_eq!((g.point_count, g.vertex_count(), g.prim_count()), (1106, 4416, 1104));
}
#[test]
fn synthetic_multi_chunk_memcpy_concatenates() {
    let f = frame(&[b"abc", b"defgh", b"i"]);
    assert_eq!(sc_decompress(&f).unwrap(), b"abcdefghi");
}
#[test]
fn lz4_overlap_and_errors() {
    let mut d = [0u8; 6];
    lz4_block(&[0x11, b'a', 1, 0], &mut d).unwrap();
    assert_eq!(&d, b"aaaaaa");
    assert!(lz4_block(&[0x11, b'a', 2, 0], &mut d).is_err()); // offset before start
    assert!(lz4_block(&[0x11, b'a', 0, 0], &mut d).is_err()); // offset 0
    assert!(lz4_block(&[0x11, b'a', 1], &mut d).is_err()); // truncated offset
    assert!(lz4_block(&[0xf0], &mut d).is_err()); // truncated length ext
    assert!(lz4_block(&[0x50, 1, 2, 3, 4, 5], &mut d).is_err()); // short output
    assert!(lz4_block(&[0x70, 1, 2, 3, 4, 5, 6, 7], &mut d).is_err()); // output overrun
    assert!(lz4_block(&[0x11, b'a', 1, 0], &mut [0u8; 5]).is_err()); // match overruns dst
}
#[test]
fn truncation_at_every_length_is_err_not_panic() {
    let s = sphere();
    for n in 0..s.len().min(4000) {
        assert!(sc_decompress(&s[..n]).is_err(), "prefix {n}");
    }
    for n in [s.len() - 1, s.len() - 28, s.len() - 29, s.len() / 2] {
        assert!(sc_decompress(&s[..n]).is_err(), "prefix {n}");
    }
}
#[test]
fn header_lies_are_syntax_or_toolarge() {
    let s = sphere();
    let patch = |at: usize, v: &[u8]| {
        let mut b = s.clone();
        b[at..at + v.len()].copy_from_slice(v);
        b
    };
    // chunk header at 12: version(12) versionlz(13) flags(14) typesize(15) nbytes(16..20) blocksize(20..24) cbytes(24..28)... see layout
    let nb = |v: u32| patch(16, &v.to_le_bytes());
    assert!(matches!(sc_decompress(&nb(23041)), Err(GeoError::Syntax(_)))); // nbytes lie (small)
    assert!(matches!(sc_decompress(&nb(23043)), Err(GeoError::Syntax(_)))); // nbytes lie (large)
    assert!(matches!(sc_decompress(&nb(u32::MAX)), Err(GeoError::TooLarge { .. }))); // > 512 MB cap
    assert!(matches!(sc_decompress(&nb(500_000_000)), Err(GeoError::Syntax(_)))); // implausible vs cbytes, no alloc
    assert!(matches!(sc_decompress(&patch(14, &[0xe1])), Err(GeoError::Syntax(_)))); // codec id 7
    assert!(matches!(sc_decompress(&patch(14, &[0x01])), Err(GeoError::Unsupported(_)))); // blosclz not implemented
    assert!(matches!(sc_decompress(&patch(14, &[0x81])), Err(GeoError::Unsupported(_)))); // zstd
    assert!(matches!(sc_decompress(&patch(14, &[0x25])), Err(GeoError::Unsupported(_)))); // bitshuffle
    assert!(matches!(sc_decompress(&patch(15, &[0])), Err(GeoError::Syntax(_)))); // typesize 0
    assert!(matches!(sc_decompress(&patch(12, &[9])), Err(GeoError::Syntax(_)))); // version
    assert!(matches!(sc_decompress(&patch(20, &0u32.to_le_bytes())), Err(GeoError::Syntax(_)))); // blocksize 0
    assert!(matches!(sc_decompress(&patch(28, &u32::MAX.to_le_bytes())), Err(GeoError::Syntax(_)))); // bstart[0] out of range
    assert!(matches!(sc_decompress(&patch(36, &u32::MAX.to_le_bytes())), Err(GeoError::Syntax(_)))); // stream csize huge
    let t = s.len() - 28;
    assert!(sc_decompress(&patch(t, &0u64.to_be_bytes())).is_err()); // chunk_size 0
    assert!(sc_decompress(&patch(t + 8, &1u64.to_be_bytes())).is_err()); // last-chunk size disagrees
    assert!(sc_decompress(&patch(t + 16, &u64::MAX.to_be_bytes())).is_err()); // idx_len absurd
    assert!(sc_decompress(&patch(t + 16, &24u64.to_be_bytes())).is_err()); // idx_len vs chunk count
    assert!(sc_decompress(&patch(0, b"scf2")).is_err());
}
#[test]
fn single_byte_flips_never_panic() {
    let s = sphere();
    for i in (0..s.len()).step_by(7) {
        let mut b = s.clone();
        b[i] ^= 0xa5;
        let _ = read_geo(&b);
    }
}
#[test]
fn nested_scf1_is_refused() {
    let inner = frame(&[b"x"]);
    let outer = frame(&[&inner]);
    assert!(matches!(read_geo(&outer), Err(GeoError::Syntax(_))));
}
