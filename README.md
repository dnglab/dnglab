# rawloader

This is a rust library to extract the raw data and some metadata from digital camera images. Given an image in a supported format and camera you will be able to get everything needed to process the image:

  * Identification of the camera that produced the image (both the EXIF name and a clean/canonical name)
  * The raw pixels themselves, exactly as encoded by the camera
  * The black and white points of each of the color channels
  * The multipliers to apply to the color channels for the white balance
  * A conversion matrix between the camera color space and XYZ
  * The description of the bayer pattern itself so you'll know which pixels are which color

Current State
-------------

The library is still in its very beginning with only the simple Minolta MRW format implemented. 

Usage
-----

Here's a simple sample program that uses this library:

```rust
use std::env;
use std::fs::File;
use std::io::prelude::*;

extern crate rawloader;
use rawloader::decoders;

fn main() {
  let args: Vec<_> = env::args().collect();
  if args.len() != 2 {
    println!("Usage: {} <file>", args[0]);
    std::process::exit(2);
  }
  let file = &args[1];
  println!("Loading file \"{}\"", file);

  let mut f = File::open(file).unwrap();
  let mut buffer = Vec::new();
  f.read_to_end(&mut buffer).unwrap();

  let rawloader = decoders::RawLoader::new();
  let decoder = rawloader.get_decoder(&buffer).unwrap();
  let camera = decoder.identify().unwrap();
  println!("Found camera \"{}\" model \"{}\"", camera.make, camera.model);
  println!("Found canonical named camera \"{}\" model \"{}\"", camera.canonical_make, camera.canonical_model);

  let image = decoder.image().unwrap();
  println!("Image size is {}x{}", image.width, image.height);
  println!("WB coeffs are {},{},{},{}", image.wb_coeffs[0],
                                        image.wb_coeffs[1],
                                        image.wb_coeffs[2],
                                        image.wb_coeffs[3]);
  println!("black levels are {:?}", image.blacklevels);
  println!("white levels are {:?}", image.whitelevels);
  println!("color matrix is {:?}", image.color_matrix);
  println!("dcraw filters {:#x}", image.dcraw_filters);

  // Write out the image as a grayscale PPM in an extremely inneficient way
  let mut f = File::create(format!("{}.ppm",file)).unwrap();
  let preamble = format!("P6 {} {} {}\n", image.width, image.height, 4095).into_bytes();
  f.write_all(&preamble).unwrap();
  for row in 0..image.height {
    let from: usize = (row as usize) * (image.width as usize);
    let to: usize = ((row+1) as usize) * (image.width as usize);
    let imgline = &image.data[from .. to];

    for pixel in imgline {
      // Do an extremely crude "demosaic" by setting R=G=B
      let bytes = [(pixel>>4) as u8, (pixel&0x0f) as u8, (pixel>>4) as u8, (pixel&0x0f) as u8, (pixel>>4) as u8, (pixel&0x0f) as u8];
      f.write_all(&bytes).unwrap();
    }
  }
}
```

Contributing
------------

Bug reports and pull requests welcome at https://github.com/pedrocr/rawloader
