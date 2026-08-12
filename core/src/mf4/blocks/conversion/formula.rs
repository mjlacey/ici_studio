use crate::mf4::blocks::conversion::base::ConversionBlock;
use crate::mf4::blocks::conversion::types::ConversionType;
use crate::mf4::blocks::common::read_string_block;
use crate::mf4::error::MdfError;

impl ConversionBlock {
    /// Resolve and store the algebraic formula referenced by this block.
    ///
    /// # Arguments
    /// * `file_data` - Memory mapped MDF bytes used to read the formula text.
    ///
    /// # Returns
    /// `Ok(())` on success or an [`MdfError`] if the formula block cannot be
    /// read.
    pub fn resolve_formula(&mut self, file_data: &[u8]) -> Result<(), MdfError> {
        if self.cc_type != ConversionType::Algebraic || self.cc_ref.is_empty() {
            return Ok(());
        }

        let addr = self.cc_ref[0];
        if let Some(formula) = read_string_block(file_data, addr)? {
            self.formula = Some(formula);
        }

        Ok(())
    }
}
