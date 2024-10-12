/// items of this type will get associated with functions of type `Aux` -> some return type
/// say they are f and g for self and other
/// would `f(my_aux)` and `g(others_aux)` have all their steps interleave
/// they are not reading/writing to any of the same locations, etc
/// some Consumers might interleave with all of the possibilities
/// like if it is only reading and none of the Consumer's associated to any of this type
///     ever write to that particular location
///     or it might not even read anything external at all with all it needs in the passed `Aux`
///     without using any reference types
pub trait Interleaves<Aux> {
    fn do_interleave(&self, my_aux: &Aux, other: &Self, others_aux: &Aux) -> bool;
    fn interleaves_with_everything(&self, my_aux: &Aux) -> bool;
}
