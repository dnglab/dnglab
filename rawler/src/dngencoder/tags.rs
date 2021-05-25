// SPDX-License-Identifier: LGPL-2.1
// Copyright by image-tiff authors (see https://github.com/image-rs/image-tiff)
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

macro_rules! tags {
    {
        // Permit arbitrary meta items, which include documentation.
        $( #[$enum_attr:meta] )*
        $vis:vis enum $name:ident($ty:tt) $(unknown($unknown_doc:literal))* {
            // Each of the `Name = Val,` permitting documentation.
            $($(#[$ident_attr:meta])* $tag:ident = $val:expr,)*
        }
    } => {
        $( #[$enum_attr] )*
        #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
        #[non_exhaustive]
        pub enum $name {
            $($(#[$ident_attr])* $tag,)*
            $(
                #[doc = $unknown_doc]
                Unknown($ty),
            )*
        }

        impl $name {
            #[inline(always)]
            fn __from_inner_type(n: $ty) -> Result<Self, $ty> {
                match n {
                    $( $val => Ok($name::$tag), )*
                    n => Err(n),
                }
            }

            #[inline(always)]
            fn __to_inner_type(&self) -> $ty {
                match *self {
                    $( $name::$tag => $val, )*
                    $( $name::Unknown(n) => { $unknown_doc; n }, )*
                }
            }
        }

        tags!($name, $ty, $($unknown_doc)*);
    };
    // For u16 tags, provide direct inherent primitive conversion methods.
    ($name:tt, u16, $($unknown_doc:literal)*) => {
        impl $name {
            #[inline(always)]
            pub fn from_u16(val: u16) -> Option<Self> {
                Self::__from_inner_type(val).ok()
            }

            $(
            #[inline(always)]
            pub fn from_u16_exhaustive(val: u16) -> Self {
                $unknown_doc;
                Self::__from_inner_type(val).unwrap_or_else(|_| $name::Unknown(val))
            }
            )*

            #[inline(always)]
            pub fn to_u16(&self) -> u16 {
                Self::__to_inner_type(self)
            }
        }
    };
    // For other tag types, do nothing for now. With concat_idents one could
    // provide inherent conversion methods for all types.
    ($name:tt, $ty:tt, $($unknown_doc:literal)*) => {};
}

// Note: These tags appear in the order they are mentioned in the TIFF reference
tags! {
/// TIFF tags
pub enum Tag(u16) unknown("A private or extension tag") {
    // Baseline tags:
    Artist = 315,
    // grayscale images PhotometricInterpretation 1 or 3
    BitsPerSample = 258,
    CellLength = 265, // TODO add support
    CellWidth = 264, // TODO add support
    // palette-color images (PhotometricInterpretation 3)
    ColorMap = 320, // TODO add support
    Compression = 259, // TODO add support for 2 and 32773
    Copyright = 33_432,
    DateTime = 306,
    ExtraSamples = 338, // TODO add support
    FillOrder = 266, // TODO add support
    FreeByteCounts = 289, // TODO add support
    FreeOffsets = 288, // TODO add support
    GrayResponseCurve = 291, // TODO add support
    GrayResponseUnit = 290, // TODO add support
    HostComputer = 316,
    ImageDescription = 270,
    ImageLength = 257,
    ImageWidth = 256,
    Make = 271,
    MaxSampleValue = 281, // TODO add support
    MinSampleValue = 280, // TODO add support
    Model = 272,
    NewSubfileType = 254, // TODO add support
    Orientation = 274, // TODO add support
    PhotometricInterpretation = 262,
    PlanarConfiguration = 284,
    ResolutionUnit = 296, // TODO add support
    RowsPerStrip = 278,
    SamplesPerPixel = 277,
    Software = 305,
    StripByteCounts = 279,
    StripOffsets = 273,
    SubfileType = 255, // TODO add support
    Threshholding = 263, // TODO add support
    XResolution = 282,
    YResolution = 283,
    // Advanced tags
    Predictor = 317,
    TileWidth = 322,
    TileLength = 323,
    TileOffsets = 324,
    TileByteCounts = 325,
    // Data Sample Format
    SampleFormat = 339,
    SMinSampleValue = 340, // TODO add support
    SMaxSampleValue = 341, // TODO add support
    // JPEG
    JPEGTables = 347,

    // TIFF-EP
    CFAPattern = 33422,
    CFARepeatPatternDim = 33421,

    SubIFDs = 0x014a,

    // EXIF?
    ExifIFD = 34665,

    ExifExposureTime = 0x829a,
    ExifFNumber = 0x829d,
    ExifISOSpeedRatings = 0x8827,
    ExifVersion = 0x9000,
    ExifLensModel = 0xa434,


    // DNG
    DNGVersion = 50706,
    DNGBackwardVersion = 50707,
    UniqueCameraModel = 50708,
    LocalizedCameraModel = 50709,
    CFAPlaneColor = 50710,
    CFALayout = 50711,
    LinearizationTable = 50712,
    BlackLevelRepeatDim = 50713,
    BlackLevel = 50714,
    BlackLevelDeltaH = 50715,
    BlackLevelDeltaV = 50716,
    WhiteLevel = 50717,
    DefaultScale = 50718,
    BestQualityScale = 50780,
    DefaultCropOrigin = 50719,
    DefaultCropSize = 50720,
    CalibrationIlluminant1 = 50778,
    CalibrationIlluminant2 = 50779,
    ColorMatrix1 = 50721,
    ColorMatrix2 = 50722,
    CameraCalibration1 = 50723,
    CameraCalibration2 = 50724,
    ReductionMatrix1 = 50725,
    ReductionMatrix2 = 50726,
    AnalogBalance = 50727,
    AsShotNeutral = 50728,
    AsShotWhiteXY = 50729,
    BaselineExposure = 50730,
    BaselineNoise = 50731,
    BaselineSharpness = 50732,
    BayerGreenSplit = 50733,
    LinearResponseLimit = 50734,
    CameraSerialNumber = 50735,
    LensInfo = 50736,
    ChromaBlurRadius = 50737,
    AntiAliasStrength = 50738,
    ShadowScale = 50739,
    DNGPrivateData = 50740,
    MakerNoteSafety = 50741,
    RawDataUniqueID = 50781,
    OriginalRawFileName = 50827,
    OriginalRawFileData = 50828,
    ActiveArea = 50829,
    MaskedAreas = 50830,
    AsShotICCProfile = 50831,
    AsShotPreProfileMatrix = 50832,
    CurrentICCProfile = 50833,
    CurrentPreProfileMatrix = 50834,
    // 1.2.0
    ColorimetricReference = 50879,
    CameraCalibrationSignature = 50931,
    ProfileCalibrationSignature = 50932,
    ExtraCameraProfiles = 50933,
    AsShotProfileName = 50934,
    NoiseReductionApplied = 50935,
    ProfileName = 50936,
    ProfileHueSatMapDims = 50937,
    ProfileHueSatMapData1 = 50938,
    ProfileHueSatMapData2 = 50939,
    ProfileToneCurve = 50940,
    ProfileEmbedPolicy = 50941,
    ProfileCopyright = 50942,
    ForwardMatrix1 = 50964,
    ForwardMatrix2 = 50965,
    PreviewApplicationName = 50966,
    PreviewApplicationVersion = 50967,
    PreviewSettingsName = 50968,
    PreviewSettingsDigest = 50969,
    PreviewColorSpace = 50970,
    PreviewDateTime = 50971,
    RawImageDigest = 50972,
    OriginalRawFileDigest = 50973,
    SubTileBlockSize = 50974,
    RowInterleaveFactor = 50975,
    ProfileLookTableDims = 50981,
    ProfileLookTableData = 50982,
    // 1.3.0
    OpcodeList1 = 51008,
    OpcodeList2 = 51009,
    OpcodeList3 = 51022,
    NoiseProfile = 51041,
    // 1.4.0
    DefaultUserCrop = 51125,
    DefaultBlackRender = 51110,
    BaselineExposureOffset = 51109,
    ProfileLookTableEncoding = 51108,
    ProfileHueSatMapEncoding = 51107,
    OriginalDefaultFinalSize = 51089,
    OriginalBestQualityFinalSize = 51090,
    OriginalDefaultCropSize = 51091,
    NewRawImageDigest = 51111,
    RawToPreviewGain = 51112,

}
}

tags! {
/// The type of an IFD entry (a 2 byte field).
#[allow(clippy::upper_case_acronyms)]
pub enum Type(u16) {
    /// 8-bit unsigned integer
    BYTE = 1,
    /// 8-bit byte that contains a 7-bit ASCII code; the last byte must be zero
    ASCII = 2,
    /// 16-bit unsigned integer
    SHORT = 3,
    /// 32-bit unsigned integer
    LONG = 4,
    /// Fraction stored as two 32-bit unsigned integers
    RATIONAL = 5,
    /// 8-bit signed integer
    SBYTE = 6,
    /// 8-bit byte that may contain anything, depending on the field
    UNDEFINED = 7,
    /// 16-bit signed integer
    SSHORT = 8,
    /// 32-bit signed integer
    SLONG = 9,
    /// Fraction stored as two 32-bit signed integers
    SRATIONAL = 10,
    /// 32-bit IEEE floating point
    FLOAT = 11,
    /// 64-bit IEEE floating point
    DOUBLE = 12,
    /// 32-bit unsigned integer (offset)
    IFD = 13,
    /// BigTIFF 64-bit unsigned integer
    LONG8 = 16,
    /// BigTIFF 64-bit signed integer
    SLONG8 = 17,
    /// BigTIFF 64-bit unsigned integer (offset)
    IFD8 = 18,
}
}

tags! {
/// See [TIFF compression tags](https://www.awaresystems.be/imaging/tiff/tifftags/compression.html)
/// for reference.
#[allow(clippy::upper_case_acronyms)]
pub enum CompressionMethod(u16) {
    None = 1,
    Huffman = 2,
    Fax3 = 3,
    Fax4 = 4,
    LZW = 5,
    JPEG = 6,
    // "Extended JPEG" or "new JPEG" style
    ModernJPEG = 7,
    Deflate = 8,
    OldDeflate = 0x80B2,
    PackBits = 0x8005,
}
}

tags! {
#[allow(clippy::upper_case_acronyms)]
pub enum PhotometricInterpretation(u16) {
    WhiteIsZero = 0,
    BlackIsZero = 1,
    RGB = 2,
    RGBPalette = 3,
    TransparencyMask = 4,
    CMYK = 5,
    YCbCr = 6,
    CIELab = 8,
    // Defined by DNG
    CFA = 32803,
    LinearRaw = 34892,
}
}

tags! {
pub enum PlanarConfiguration(u16) {
    Chunky = 1,
    Planar = 2,
}
}

tags! {
pub enum Predictor(u16) {
    None = 1,
    Horizontal = 2,
}
}

tags! {
/// Type to represent resolution units
pub enum ResolutionUnit(u16) {
    None = 1,
    Inch = 2,
    Centimeter = 3,
}
}

tags! {
#[allow(clippy::upper_case_acronyms)]
pub enum SampleFormat(u16) unknown("An unknown extension sample format") {
    Uint = 1,
    Int = 2,
    IEEEFP = 3,
    Void = 4,
}
}
