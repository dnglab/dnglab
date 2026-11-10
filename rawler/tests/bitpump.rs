use bitstream_io::{BigEndian, BitRead, BitReader};
use rawler::pumps::{BitPump, BitPumpError, BitPumpMSB};
use std::io::{Cursor, ErrorKind};

/// Test data with a given chance of 1-bits, to get short and long runs of zero bits
fn test_data(len: usize, mut state: u64, one_bits_per_256: u64) -> Vec<u8> {
  let mut next = move || {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    state
  };
  (0..len).map(|_| (0..8).fold(0_u8, |byte, _| (byte << 1) | ((next() & 0xff) < one_bits_per_256) as u8)).collect()
}

/// BitPumpMSB gives the same results as bitstream_io for any mix
/// of reads, including at the end of the buffer
#[test]
fn msb_matches_bitstream_io() {
  for len in (0..40).chain([100, 4097]) {
    for (seed, density) in [(0x9e3779b97f4a7c15, 128), (0xd1b54a32d192ed03, 16), (0xa0761d6478bd642f, 1), (0x8ebc6af09c88c6e3, 0)] {
      let data = test_data(len, seed, density);
      let mut reader = BitReader::endian(Cursor::new(data.as_slice()), BigEndian);
      let mut pump = BitPumpMSB::new(&data);
      let mut op = seed;
      loop {
        op = op.wrapping_mul(0x2545f4914f6cdd1d).wrapping_add(1);
        let num = ((op >> 40) % 34) as u32;
        let (expected, actual) = match op >> 62 {
          // A unary code is a run of zero bits and the 1-bit after it
          0 | 1 => (reader.read_unary::<1>(), pump.consume_zerobits().inspect(|_| pump.consume_bits(1))),
          2 => (reader.read_var::<u32>(num), pump.get_bits(num)),
          // Peeking must not consume
          _ => (reader.read_var::<u32>(num), pump.peek_bits(num).and_then(|_| pump.get_bits(num))),
        };
        match (expected, actual) {
          (Ok(expected), Ok(actual)) => assert_eq!(expected, actual, "len {len}, density {density}"),
          (Err(expected), Err(actual)) => {
            let kind = match actual {
              BitPumpError::Eof => ErrorKind::UnexpectedEof,
              BitPumpError::ExcessiveBits { .. } => ErrorKind::InvalidInput,
            };
            assert_eq!(expected.kind(), kind, "len {len}, density {density}");
            break;
          }
          (expected, actual) => panic!("len {len}, density {density}: {expected:?} != {actual:?}"),
        }
      }
    }
  }
}

#[test]
fn msb_too_many_bits_is_an_error() {
  let data = [0xff; 16];
  let excessive = Err(BitPumpError::ExcessiveBits { num: 33, max: 32 });
  assert_eq!(BitPumpMSB::new(&data).peek_bits(33), excessive);
  // Also when the bit register already holds that many bits
  let mut pump = BitPumpMSB::new(&data);
  assert_eq!(pump.get_bits(1), Ok(1));
  assert_eq!(pump.peek_bits(33), excessive);
  assert_eq!(BitPumpMSB::new(&data).get_bits(32), Ok(u32::MAX));
  assert_eq!(BitPumpMSB::new(&data).peek_bits(0), Ok(0));
  // At the end of the buffer it is still not an Eof
  assert_eq!(BitPumpMSB::new(&data[..2]).peek_bits(33), excessive);
}

/// A run of zero bits may end in the last bits of the buffer
#[test]
fn msb_zerobits_at_end_of_buffer() {
  // 20 zero bits, a 1-bit and 3 more bits
  let data = [0x00, 0x00, 0x0d];
  let mut pump = BitPumpMSB::new(&data);
  assert_eq!(pump.consume_zerobits(), Ok(20));
  assert_eq!(pump.get_bits(4), Ok(0b1101));
  assert_eq!(pump.peek_bits(1), Err(BitPumpError::Eof));

  // A run which is longer than the bit register
  let mut data = [0_u8; 40];
  data[39] = 0x01;
  let mut pump = BitPumpMSB::new(&data);
  assert_eq!(pump.consume_zerobits(), Ok(319));
  assert_eq!(pump.get_bits(1), Ok(1));
}

/// A run of zero bits without a 1-bit is an error
#[test]
fn msb_zerobits_without_end() {
  for len in 0..20 {
    let data = vec![0_u8; len];
    assert_eq!(BitPumpMSB::new(&data).consume_zerobits(), Err(BitPumpError::Eof), "len {len}");
  }
}
