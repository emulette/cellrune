use super::super::runtime::Array;
use super::super::value::Value;

#[derive(Debug, Clone, Copy)]
pub(super) struct VectorView<'array> {
    array: &'array Array,
    length: u32,
    vertical: bool,
}

impl<'array> VectorView<'array> {
    pub(super) fn new(array: &'array Array) -> Option<Self> {
        if array.cols == 1 {
            Some(Self {
                array,
                length: array.rows,
                vertical: true,
            })
        } else if array.rows == 1 {
            Some(Self {
                array,
                length: array.cols,
                vertical: false,
            })
        } else {
            None
        }
    }

    pub(super) const fn len(self) -> u32 {
        self.length
    }

    pub(super) fn at(self, offset: u32) -> &'array Value {
        if self.vertical {
            self.array.at(offset, 0)
        } else {
            self.array.at(0, offset)
        }
    }
}
