extern double jamscript_m0_scalar_entry(double left, double right);

int main(void) {
  return jamscript_m0_scalar_entry(1.0, 2.0) == 3.0 ? 0 : 1;
}
