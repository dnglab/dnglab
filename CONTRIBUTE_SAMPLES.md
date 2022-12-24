# How to contribute RAW samples

New raw image formats or cameras can only be integrated when a full set of raw sample files
exists. You can help to speed up support for your new camera if you contribute a full set of samples.
Please see the specific manufacturer section for further instructions.

# Important notes
For every sample set, the scene must be properly exposed during daylight. The ideal scene is a landscape
where many colors are visible, like a blue sky with white clouds, green grass and flowers/trees. Don't capture
people, cars or any other private property.

**No color checker cards, please. They won't work well for visual checks.**

The sample set must be licensed under CC-0 license. If you submit a sample set, you explicitly state that
you are the copyright owner of the images.

**Please don't submit files with names like IMG_0001.CR2. Always give your files a good name like CANON_5DMK3_ISO_100_SRAW.CR2.**

# How to upload
As the samples sets are usually very big, please use an upload service like [wetransfer.com]. After uploading as a ZIP file,
please raise an issue on our [issue tracker](https://github.com/dnglab/dnglab/issues). and provide the download link.


# Instructions

## Canon (CR3)

Before you do the shots, please go to menu: https://cam.start.canon/en/C006/manual/html/UG-09_Set-up_0300.html and set copyright to: "CC-0", so we can use the sample set in all raw sample databases.

New Canon cameras (>=2019) may have some special capture modes. If your camera supports different shutter modes (mechanical, electrical),
please duplicate the core set and take one with mechanical, another one with electrical shutter (the latter uses reduced bit depth).

The core set must consists of these shoots:
 * **RAW** mode: ISO 100, 800, MAX-ISO (depends on your camera, often denotes by H or H1/H2)
 * **CRAW** mode: ISO 100, 800, MAX-ISO (depends on your camera, often denotes by H or H1/H2)

If your camera supports *burst mode*:
 * **Burst** mode: ISO 100 with 3-5 frames maximum (keep the size small)

If your camera supports *HEIF* format:
 * **HEIF** mode: ISO 100

## Olympus / OM Digial Solutions (ORF)

The core set must consists of these shoots:
 * ISO 100

If your camera supports high resolutions modes:
 * ISO 100 with high resolution mode (handheld, tripod, ...)


## Fujifilm

The core set must consists of these shoots (if supported):
 * **uncompressed** mode: ISO 100
 * **lossless compressed** mode: ISO 100
 * **lossy compressed** mode: ISO 100

## Panasonic

The core set must consists of these shoots (if supported):
 * ISO 100 with all possible crops (1:1, 4:3, 3:2, 16:9)


## Phase One / Leaf
The core set must consists of these shoots (if supported):
 * **IIQ-L**mode: ISO 100
 * **IIQ-S**mode: ISO 100
 * **IIQ-Sv2**mode: ISO 100

## Pentax
The core set must consists of these shoots (if supported):
 * **PEF** mode: ISO 100
 * **DNG** mode: ISO 100

If your camera supports high resolution / pxiel shift modes:
 * PEF and DNG with high resolution mode

## Nikon and Sony (NEF/ARW)

The core set must consists of all the combinations of compression and bitness settings, plus all the raw sizes, that is:

 * 12bit-compressed
 * 12bit-uncompressed
 * 12bit-lossless-compressed
 * 14bit-compressed
 * 14bit-uncompressed
 * 14bit-lossless-compressed
 * L (large), M (medium), S (small) formats
 * APS-C S35 mode
