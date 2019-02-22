#[macro_use]
extern crate afl;
extern crate rawloader;

fn main() {
  rawloader::force_initialization();

  fuzz!(|data: &[u8]| {
    // Remove the panic hook so we can actually catch panic
    std::panic::set_hook(Box::new(|_| {} ));

    rawloader::decode(&mut &data[..]).ok();
  });
}
