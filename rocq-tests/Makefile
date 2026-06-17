# Build the peony Rocq/Coq formalization.
#
#   make            # compile all proofs (fails loudly on any unproved goal)
#   make clean      # remove build artifacts
#
# Requires the Rocq Prover (coqc) >= 9.0, or Coq >= 8.18.

COQC ?= coqc
VFILES := \
  SymbolLattice.v \
  RelocDisjoint.v \
  RelocMonoid.v \
  SectionGC.v \
  Layout.v \
  IncrementalSoundness.v \
  IncrementalCostBound.v \
  ParallelSchedule.v \
  ICFSoundness.v

VOFILES := $(VFILES:.v=.vo)

.PHONY: all clean check
all: $(VOFILES)

# Each .v compiles independently (no cross-file imports), so a simple rule
# suffices. coqc exits non-zero on any incomplete proof, so `make` is the
# pass/fail oracle for the whole theory.
%.vo: %.v
	$(COQC) -Q . Peony $<

check: all
	@echo "All peony proofs compiled successfully."

clean:
	rm -f *.vo *.vok *.vos *.glob .*.aux
