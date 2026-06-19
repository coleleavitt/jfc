# JFC Formal Proofs Makefile
# Requires Coq/Rocq 8.18+ with coq_makefile and coqchk.

COQMAKEFILE := Makefile.coq
COQMAKEFILE_BIN ?= coq_makefile
COQCHK ?= coqchk

.PHONY: all clean theories extraction check

all: $(COQMAKEFILE)
	$(MAKE) -f $(COQMAKEFILE)

$(COQMAKEFILE): _CoqProject
	$(COQMAKEFILE_BIN) -f _CoqProject -o $@

theories: all

extraction: all
	@echo "Extraction targets not yet configured"

clean:
	-$(MAKE) -f $(COQMAKEFILE) clean 2>/dev/null || true
	rm -f $(COQMAKEFILE) $(COQMAKEFILE).conf
	rm -f theories/*.vo theories/*.vok theories/*.vos theories/*.glob
	rm -f .*.aux

install: all
	$(MAKE) -f $(COQMAKEFILE) install

check: all
	@command -v $(COQCHK) >/dev/null 2>&1 || { \
		echo "$(COQCHK) not found; install Coq/Rocq with coqchk for kernel checking." >&2; \
		exit 127; \
	}
	@mods="$$(sed -n 's/[[:space:]]*#.*//; /^[[:space:]]*$$/d; /\.v[[:space:]]*$$/p' _CoqProject \
		| sed -e 's/^[[:space:]]*//' \
		      -e 's/[[:space:]]*$$//' \
		      -e 's#^theories/#JFC.#' \
		      -e 's#^theorems/#JFCTheorems.#' \
		      -e 's#/#.#g' \
		      -e 's/\.v$$//')"; \
	if [ -z "$$mods" ]; then \
		echo "No Coq modules found in _CoqProject" >&2; \
		exit 1; \
	fi; \
	echo "Kernel-checking proofs with $(COQCHK)"; \
	$(COQCHK) -R theories JFC -R theorems JFCTheorems $$mods
	@echo "All proofs kernel-checked."
