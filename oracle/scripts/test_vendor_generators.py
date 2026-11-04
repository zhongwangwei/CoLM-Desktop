#!/usr/bin/env python3
"""Exercise real make expansion and collective-safe history field generation."""
from contextlib import redirect_stderr
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]


def load(relative):
    source = ROOT / relative
    module = types.ModuleType(source.stem)
    module.__file__ = str(source)
    exec(compile(source.read_text(), str(source), 'exec'), module.__dict__)
    return module


class DependencyExpansion(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='colm-deps-test-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.deps = load('oracle/scripts/gen_fortran_deps.py')
        self.deps.VENDOR = str(self.root)
        self.deps.MAKEFILE = str(self.root / 'Makefile')
        self.text = '''A = a.o
B = b.o
EMPTY =
ifeq (0,1)
INACTIVE = unused.o
endif
VPATH = .
$(A) $(B): %.o: %.F90
'''
        (self.root / 'Makefile').write_text(self.text)
        (self.root / 'a.F90').write_text('module ma\nuse mb\nend module\n')
        (self.root / 'b.F90').write_text('module mb\nend module\n')

    def test_combined_phase_keeps_cross_variable_module_dependencies(self):
        self.assertEqual(self.deps.stage_variables(self.text), ['$(A) $(B)'])
        with redirect_stderr(io.StringIO()):
            _, block = self.deps.generate()
        self.assertIn('a.o: b.o\n', block)
        self.assertIn('$(A) $(B): | mkdir_build', block)
        self.assertEqual(self.deps.expand('$(filter-out b.o,$(A) $(B))'), ['a.o'])

    def test_empty_and_inactive_defined_phases_remain_legal(self):
        self.assertEqual(self.deps.expand('EMPTY'), [])
        self.assertEqual(self.deps.expand('INACTIVE'), [])

    def test_unknown_phase_has_variable_and_makefile_context(self):
        with self.assertRaises(RuntimeError) as error:
            self.deps.expand('MISSING_PHASE')
        self.assertIn('MISSING_PHASE', str(error.exception))
        self.assertIn(str(self.root / 'Makefile'), str(error.exception))

    def test_unknown_nested_variable_is_not_a_legal_empty_phase(self):
        (self.root / 'Makefile').write_text(self.text + '\nBROKEN = a.o $(MISSING_NESTED)\n')
        with redirect_stderr(io.StringIO()):
            with self.assertRaises(RuntimeError) as error:
                self.deps.expand('BROKEN')
        self.assertIn('MISSING_NESTED', str(error.exception))
        self.assertIn('BROKEN', str(error.exception))
        self.assertIn(str(self.root / 'Makefile'), str(error.exception))

    def test_missing_or_invalid_make_output_is_rejected(self):
        result = subprocess.CompletedProcess([], 0, stdout='', stderr='')
        with patch.object(self.deps.subprocess, 'run', return_value=result):
            with self.assertRaisesRegex(RuntimeError, 'missing expansion output'):
                self.deps.expand('A')
        (self.root / 'Makefile').write_text(self.text + '\nBAD = not-an-object\n')
        with self.assertRaisesRegex(RuntimeError, 'non-object expansion'):
            self.deps.expand('BAD')


class HistoryFields(unittest.TestCase):
    def test_runtime_guards_preserve_collective_participation(self):
        history = load('vendor/CoLM202X/.github/workflows/hist_acc_restart_fields.py')
        with tempfile.TemporaryDirectory(prefix='colm-history-fields-') as directory:
            allocation = Path(directory) / 'allocate.F90'
            allocation.write_text('''SUBROUTINE allocate_acc_fluxes
IF (p_is_worker) THEN
 IF (numpatch > 0) THEN
  allocate (a_base(numpatch))
  IF (numurban > 0) THEN
   allocate (a_city(numurban))
  ENDIF
  IF (DEF_USE_BGC) THEN
   allocate (a_carbon(2,numpatch))
  ENDIF
  IF (mt_any_enabled()) THEN
   allocate (nac_frcsat(numpatch))
  ENDIF
 ENDIF
ENDIF
END SUBROUTINE allocate_acc_fluxes
''')
            history.ACC = str(allocation)
            rendered = '\n'.join(history.render(history.fields()))
        self.assertIn('      IF (DEF_URBAN_RUN) THEN\n         CALL hacc_field1d', rendered)
        self.assertIn('      IF (DEF_USE_BGC) THEN\n         CALL hacc_field2d', rendered)
        self.assertNotIn('p_is_worker', rendered)
        self.assertNotIn('numpatch > 0', rendered)
        self.assertIn('HACC_WRITE, nac, a_frcsat, nac_frcsat, &', rendered)

    def test_checked_in_field_program_matches_current_allocations(self):
        script = ROOT / 'vendor/CoLM202X/.github/workflows/hist_acc_restart_fields.py'
        for directory in [ROOT, ROOT / 'vendor/CoLM202X']:
            with self.subTest(directory=directory):
                result = subprocess.run(
                    [sys.executable, str(script), '--check'],
                    cwd=directory, capture_output=True, text=True, timeout=20)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
