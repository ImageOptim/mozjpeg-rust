use fallible_collections::FallibleVec;

pub trait VecUninitExtender {
    unsafe fn extend_uninit(&mut self, items: usize);
}

impl<T: Copy> VecUninitExtender for Vec<T> {
    unsafe fn extend_uninit(&mut self, items: usize) {
        let new_len = self.len() + items;
        FallibleVec::try_reserve(self, items).expect("oom");
        self.set_len(new_len);
    }
}
