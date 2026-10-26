pub mod commands;
pub mod error;
pub mod fs;
mod utils;

pub use commands::add::{add_file, AddedFile};
pub use commands::list::list_files;
pub use error::{AddError, IoOperation};
