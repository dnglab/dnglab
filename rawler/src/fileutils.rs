// SPDX-License-Identifier: LGPL-2.1
// Copyright 2026 Daniel Vogelbacher <daniel@chaospixel.com>

//! Helpers for generated file names

/// Characters allowed in file names besides ASCII letters and digits
const ALLOWED_CHARS: [char; 6] = [' ', '-', '_', '.', '(', ')'];

/// File name of `name` that is valid on all platforms
///
/// Only ASCII letters and digits and the characters of [`ALLOWED_CHARS`]
/// are kept, others are replaced by '_'. Leading and trailing spaces and
/// dots are removed, the result can be empty.
pub(crate) fn sanitize(name: &str) -> String {
  let name: String = name
    .chars()
    .map(|c| if c.is_ascii_alphanumeric() || ALLOWED_CHARS.contains(&c) { c } else { '_' })
    .collect();
  name.trim_matches(|c| c == ' ' || c == '.').to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn sanitize_file_names() {
    assert_eq!(sanitize("Canon EOS 100D Standard"), "Canon EOS 100D Standard");
    assert_eq!(sanitize("A/B: C?"), "A_B_ C_");
    assert_eq!(sanitize("Portrait (v2) ä+b\tc. "), "Portrait (v2) __b_c");
    assert_eq!(sanitize(" .hidden.dcp"), "hidden.dcp");
    assert_eq!(sanitize(" . "), "");
  }
}
