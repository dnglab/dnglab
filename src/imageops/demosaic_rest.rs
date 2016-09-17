//Patterned Pixel Grouping Interpolation by Alain Desbiolles
pub fn ppg_interpolate(img: &Image) -> Vec<f32> {
  let mut out: Vec<f32> = vec![0; (img.width*img.height*4) as usize];

  let dir: [i32; 5] = { 1, img.width, -1, -img.width, 1 };
  int row, col, diff[2], guess[2], c, d, i;
  ushort (*pix)[4];

  {
    // First interpolate just the borders
    border_interpolate(3, img, &mut out);
  }

  /*  Fill in the green layer with gradients and pattern recognition: */
  for (row=3; row < height-3; row++)
    for (col=3+(FC(row,3) & 1), c=FC(row,col); col < width-3; col+=2) {
      pix = image + row*width+col;
      for (i=0; (d=dir[i]) > 0; i++) {
        guess[i] = (pix[-d][1] + pix[0][c] + pix[d][1]) * 2
                      - pix[-2*d][c] - pix[2*d][c];
        diff[i] = ( ABS(pix[-2*d][c] - pix[ 0][c]) +
                    ABS(pix[ 2*d][c] - pix[ 0][c]) +
                    ABS(pix[  -d][1] - pix[ d][1]) ) * 3 +
                  ( ABS(pix[ 3*d][1] - pix[ d][1]) +
                    ABS(pix[-3*d][1] - pix[-d][1]) ) * 2;
      }
      d = dir[i = diff[0] > diff[1]];
      pix[0][1] = ULIM(guess[i] >> 2, pix[d][1], pix[-d][1]);
    }
/*  Calculate red and blue for each green pixel:                */
  for (row=1; row < height-1; row++)
    for (col=1+(FC(row,2) & 1), c=FC(row,col+1); col < width-1; col+=2) {
      pix = image + row*width+col;
      for (i=0; (d=dir[i]) > 0; c=2-c, i++)
        pix[0][c] = CLIP((pix[-d][c] + pix[d][c] + 2*pix[0][1]
                        - pix[-d][1] - pix[d][1]) >> 1);
    }
/*  Calculate blue for red pixels and vice versa:                */
  for (row=1; row < height-1; row++)
    for (col=1+(FC(row,1) & 1), c=2-FC(row,col); col < width-1; col+=2) {
      pix = image + row*width+col;
      for (i=0; (d=dir[i]+dir[i+1]) > 0; i++) {
        diff[i] = ABS(pix[-d][c] - pix[d][c]) +
                  ABS(pix[-d][1] - pix[0][1]) +
                  ABS(pix[ d][1] - pix[0][1]);
        guess[i] = pix[-d][c] + pix[d][c] + 2*pix[0][1]
                 - pix[-d][1] - pix[d][1];
      }
      if (diff[0] != diff[1])
        pix[0][c] = CLIP(guess[diff[0] > diff[1]] >> 1);
      else
        pix[0][c] = CLIP((guess[0]+guess[1]) >> 2);
    }
}

