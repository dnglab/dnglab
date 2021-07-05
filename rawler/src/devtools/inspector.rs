  // The debug version
  #[cfg(feature = "inspector")]
  #[macro_export]
  macro_rules! inspector {
      ($( $args:expr ),*) => { println!( $( $args ),* ); }
  }

  // Non-debug version
  #[cfg(not(feature = "inspector"))]
  #[macro_export]
  macro_rules! inspector {
      ($( $args:expr ),*) => {}
  }
