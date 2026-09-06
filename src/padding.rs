use rand::RngCore;

/// Padding logic ported directly from Xray-core (Project X).
/// Adds randomized noise/alignment to outgoing DNS-over-HTTPS packets
/// to obfuscate packet length signatures and bypass DPI fingerprinting.
///
/// Original Go implementation: https://github.com/XTLS/Xray-core

const CHARSET_BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

static HPACK_HUFFMAN_BIT_LENGTHS: [u32; 256] = {
    let mut t = [30u32; 256];

    let codes: &[(usize, u32)] = &[
        (0, 13),
        (1, 23),
        (2, 28),
        (3, 28),
        (4, 28),
        (5, 28),
        (6, 28),
        (7, 28),
        (8, 28),
        (9, 24),
        (10, 30),
        (11, 28),
        (12, 28),
        (13, 30),
        (14, 28),
        (15, 28),
        (16, 28),
        (17, 28),
        (18, 28),
        (19, 28),
        (20, 28),
        (21, 28),
        (22, 30),
        (23, 28),
        (24, 28),
        (25, 28),
        (26, 28),
        (27, 28),
        (28, 28),
        (29, 28),
        (30, 28),
        (31, 28),
        (32, 6),
        (33, 10),
        (34, 10),
        (35, 12),
        (36, 13),
        (37, 6),
        (38, 13),
        (39, 13),
        (40, 11),
        (41, 11),
        (42, 13),
        (43, 13),
        (44, 10),
        (45, 6),
        (46, 8),
        (47, 11),
        (48, 5),
        (49, 5),
        (50, 5),
        (51, 6),
        (52, 6),
        (53, 6),
        (54, 6),
        (55, 6),
        (56, 6),
        (57, 6),
        (58, 7),
        (59, 13),
        (60, 15),
        (61, 6),
        (62, 12),
        (63, 10),
        (64, 13),
        (65, 6),
        (66, 7),
        (67, 7),
        (68, 7),
        (69, 6),
        (70, 8),
        (71, 8),
        (72, 8),
        (73, 6),
        (74, 11),
        (75, 11),
        (76, 7),
        (77, 7),
        (78, 7),
        (79, 7),
        (80, 7),
        (81, 11),
        (82, 7),
        (83, 6),
        (84, 6),
        (85, 8),
        (86, 8),
        (87, 11),
        (88, 8),
        (89, 8),
        (90, 8),
        (91, 26),
        (92, 28),
        (93, 26),
        (94, 26),
        (95, 7),
        (96, 28),
        (97, 5),
        (98, 7),
        (99, 6),
        (100, 6),
        (101, 5),
        (102, 7),
        (103, 7),
        (104, 7),
        (105, 5),
        (106, 11),
        (107, 11),
        (108, 6),
        (109, 7),
        (110, 6),
        (111, 5),
        (112, 7),
        (113, 11),
        (114, 6),
        (115, 5),
        (116, 5),
        (117, 7),
        (118, 8),
        (119, 11),
        (120, 8),
        (121, 8),
        (122, 9),
        (123, 26),
        (124, 15),
        (125, 26),
        (126, 26),
        (127, 28),
    ];

    let mut i = 0;
    while i < codes.len() {
        let (idx, bits) = codes[i];
        t[idx] = bits;
        i += 1;
    }
    t
};

pub fn hpack_huffman_encode_len(s: &str) -> usize {
    let bits: u64 = s
        .bytes()
        .map(|b| HPACK_HUFFMAN_BIT_LENGTHS[b as usize] as u64)
        .sum();
    ((bits + 7) / 8) as usize
}

#[derive(Clone, Debug)]
pub enum PaddingMethod {
    RepeatX,
    Tokenish,
}

#[derive(Clone, Debug)]
pub enum PaddingPlacement {
    Header { header_name: String },
    Cookie { cookie_name: String },
}

fn rand_base62_string(n: usize) -> Option<String> {
    if n == 0 {
        return None;
    }
    let m = CHARSET_BASE62.len();
    let limit = 256 - (256 % m);

    let mut result = Vec::with_capacity(n);
    let mut buf = [0u8; 256];

    while result.len() < n {
        rand::thread_rng().fill_bytes(&mut buf);
        for &rb in &buf {
            if (rb as usize) >= limit {
                continue;
            }
            result.push(CHARSET_BASE62[(rb as usize) % m]);
            if result.len() == n {
                break;
            }
        }
    }

    Some(String::from_utf8(result).ok()?)
}

pub fn generate_tokenish_padding(target_huffman_bytes: usize) -> String {
    const AVG_HUFFMAN_BYTES_PER_CHAR: f64 = 0.8;
    const TOLERANCE: usize = 2;
    const MAX_ITER: usize = 150;

    let initial_n =
        ((target_huffman_bytes as f64 / AVG_HUFFMAN_BYTES_PER_CHAR).ceil() as usize).max(1);

    let mut s = match rand_base62_string(initial_n) {
        Some(s) => s,
        None => return "X".repeat(target_huffman_bytes),
    };

    let mut adjust = b'X';

    for _ in 0..MAX_ITER {
        let current = hpack_huffman_encode_len(&s);
        let diff = current as isize - target_huffman_bytes as isize;

        if diff.unsigned_abs() <= TOLERANCE {
            return s;
        }

        if diff < 0 {
            s.push(adjust as char);
            adjust = if adjust == b'X' { b'Z' } else { b'X' };
        } else {
            if s.len() <= 1 {
                return s;
            }
            s.pop();
        }
    }

    s
}

pub fn generate_padding(method: &PaddingMethod, length: usize) -> String {
    if length == 0 {
        return String::new();
    }
    match method {
        PaddingMethod::RepeatX => "X".repeat(length),
        PaddingMethod::Tokenish => {
            let s = generate_tokenish_padding(length);
            if s.is_empty() { "X".repeat(length) } else { s }
        }
    }
}

pub fn build_padding_header(
    method: PaddingMethod,
    placement: PaddingPlacement,
    length: usize,
) -> Option<(String, String)> {
    let value = generate_padding(&method, length);
    if value.is_empty() {
        return None;
    }

    match placement {
        PaddingPlacement::Header { header_name } => Some((header_name, value)),
        PaddingPlacement::Cookie { cookie_name } => {
            Some(("Cookie".to_string(), format!("{}={}", cookie_name, value)))
        }
    }
}
