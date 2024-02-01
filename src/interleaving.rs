pub trait Interleaves<Aux> {
    fn do_interleave(&self, my_aux: &Aux, other: &Self, others_aux: &Aux) -> bool;
    fn interleaves_with_everything(&self, my_aux: &Aux) -> bool;
}
