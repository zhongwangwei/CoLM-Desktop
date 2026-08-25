#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TMPDIR="$(mktemp -d "${TMPDIR:-/tmp}/colm-newcase-contract.XXXXXX")"
trap 'rm -rf "${TMPDIR}"' EXIT

if [ "$(uname -s)" = "Darwin" ]; then
  mkdir -p "${TMPDIR}/bin"
  cat >"${TMPDIR}/bin/sed" <<'SED'
#!/bin/bash
if [ "$1" = "-i" ]; then
  shift
  exec /usr/bin/sed -i '' "$@"
fi
exec /usr/bin/sed "$@"
SED
  chmod +x "${TMPDIR}/bin/sed"
  export PATH="${TMPDIR}/bin:${PATH}"
fi

bash -n "${SCRIPT_DIR}/create_newcase"
bash -n "${SCRIPT_DIR}/create_namelist"

BAD_SITELIST="${TMPDIR}/SiteList.bad"
printf 'BadSite 2001 2002 1 2 only-six\n' >"${BAD_SITELIST}"
if bash "${SCRIPT_DIR}/create_namelist" \
  -p "${TMPDIR}/BadNamelist" -n input_BadNamelist.nml \
  -d /rawdata -r /runtime -s BadSite -L "${BAD_SITELIST}" >/dev/null 2>&1; then
  echo "create_namelist accepted a malformed SiteList" >&2
  exit 1
fi
if [ -e "${TMPDIR}/BadNamelist/input_BadNamelist.nml" ]; then
  echo "create_namelist wrote output before rejecting a malformed SiteList" >&2
  exit 1
fi

if (
  cd "${SCRIPT_DIR}"
  bash ./create_newcase \
    -n "${TMPDIR}/BadNewcase" -g single_BadSite -s pc -m vg \
    -L "${BAD_SITELIST}" >/dev/null 2>&1
); then
  echo "create_newcase accepted a malformed SiteList" >&2
  exit 1
fi
if [ -e "${TMPDIR}/BadNewcase" ]; then
  echo "create_newcase created a case before rejecting a malformed SiteList" >&2
  exit 1
fi

if (
  cd "${TMPDIR}"
  bash "${ROOT}/.github/workflows/create_defineh.bash" SinglePoint LULC_IGBP CaMaOFF INVALID >/dev/null 2>&1
); then
  echo "create_defineh.bash accepted an invalid compile-time option" >&2
  exit 1
fi

mkdir -p "${TMPDIR}/bld/include"
(
  cd "${TMPDIR}/bld"
  bash "${ROOT}/.github/workflows/create_defineh.bash" SinglePoint LULC_IGBP CaMaOFF CROPON >/dev/null
)
grep -q '#define SinglePoint' "${TMPDIR}/bld/include/define.h"
grep -q '#define LULC_IGBP' "${TMPDIR}/bld/include/define.h"
grep -q '#define CROP' "${TMPDIR}/bld/include/define.h"
grep -q '#define  CatchLateralFlow' "${TMPDIR}/bld/include/define.h"
if grep -Eq '^#(define|undef)[[:space:]]+LATERAL_FLOW$' "${TMPDIR}/bld/include/define.h"; then
  echo "define.h used the nonexistent LATERAL_FLOW macro" >&2
  exit 1
fi
if grep -Eq '^#(define|undef)[[:space:]]+(LULC_IGBP_PFT|LULC_IGBP_PC|BGC|LULCC)$' "${TMPDIR}/bld/include/define.h"; then
  echo "stale compile-time runtime macro leaked into define.h" >&2
  exit 1
fi

mkdir -p "${TMPDIR}/CasePC"
COLM_DEF_USE_LCT=.false. \
COLM_DEF_USE_PFT=.false. \
COLM_DEF_USE_PC=.true. \
COLM_DEF_USE_BGC=.true. \
COLM_DEF_URBAN_RUN=.true. \
COLM_DEF_USE_LULCC=.false. \
COLM_DEF_USE_Campbell_SOIL_MODEL=.true. \
bash "${SCRIPT_DIR}/create_namelist" \
  -p "${TMPDIR}/CasePC" -n input_CasePC.nml \
  -t 2001 -e 2002 -d /rawdata -r /runtime -f CRUJRA \
  -S -90 -N 90 -W -180 -E 180 -x 1 -y 1 -g 1 >/dev/null 2>/dev/null

NML="${TMPDIR}/CasePC/input_CasePC.nml"
grep -q 'DEF_USE_LCT = .false.' "$NML"
grep -q 'DEF_USE_PC = .true.' "$NML"
grep -q 'DEF_USE_BGC = .true.' "$NML"
grep -q 'DEF_URBAN_RUN = .true.' "$NML"
grep -q 'DEF_USE_Campbell_SOIL_MODEL = .true.' "$NML"

# Expert land-cover scalar overrides must be accepted by nl_colm, validated, broadcast,
# and applied only through the SinglePoint SITE_landtype sparse overlay.
grep -q 'DEF_LC_VMAX25' "${ROOT}/share/MOD_Namelist.F90"
grep -q 'DEF_LC_C3C4' "${ROOT}/share/MOD_Namelist.F90"
grep -q 'DEF_LC_PSI50_ROOT' "${ROOT}/share/MOD_Namelist.F90"
if grep -q 'DEF_LC_UNSET' "${ROOT}/share/MOD_Namelist.F90"; then
  echo "DEF_LC_UNSET must not look like a schema/nml field" >&2
  exit 1
fi
grep -q 'real(r8) :: DEF_LC_VMAX25 = -1.e36_r8' "${ROOT}/share/MOD_Namelist.F90"
grep -q 'CALL check_lc_override' "${ROOT}/share/MOD_Namelist.F90"
grep -q 'CALL mpi_bcast (DEF_LC_VMAX25' "${ROOT}/share/MOD_Namelist.F90"
grep -q 'CALL apply_lc_scalar_overrides' "${ROOT}/main/MOD_Const_LC.F90"
grep -Fq 'vmax25     (lc) = DEF_LC_VMAX25 * 1.e-6_r8' "${ROOT}/main/MOD_Const_LC.F90"
grep -q '#ifdef SinglePoint' "${ROOT}/main/MOD_Const_LC.F90"
grep -q 'IF (.not. DEF_USE_LCT) RETURN' "${ROOT}/main/MOD_Const_LC.F90"

grep -q 'create_defineh.bash' "${SCRIPT_DIR}/create_newcase"
grep -q 'failed to generate bld/include/define.h' "${SCRIPT_DIR}/create_newcase"
if grep -q 'LULCC.*URBAN' "${SCRIPT_DIR}/create_newcase"; then
  echo "regional IGBP/PFT/PC LULCC must remain compatible with URBAN" >&2
  exit 1
fi
if grep -Eq '^\$IsDef(PFT|PC|BGC|LuLCC|Urban|CB|VG)[[:space:]]+' "${SCRIPT_DIR}/create_newcase"; then
  echo "create_newcase still writes deprecated runtime compile macros directly" >&2
  exit 1
fi

grep -q "CASE ('off', '126', '245', '370', '585')" "${ROOT}/share/MOD_Namelist.F90"
grep -q "Fatal ERROR: unknown DEF_SSP=" "${ROOT}/main/MOD_MonthlyinSituCO2MaunaLoa.F90"
if grep -q 'unknown DEF_SSP; future CO2 held' "${ROOT}/main/MOD_MonthlyinSituCO2MaunaLoa.F90"; then
  echo "unknown DEF_SSP still silently falls back to fixed CO2" >&2
  exit 1
fi

echo "create_newcase/create_namelist runtime contract ok"
