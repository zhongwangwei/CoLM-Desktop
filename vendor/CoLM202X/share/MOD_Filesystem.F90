MODULE MOD_Filesystem
   USE, INTRINSIC :: ISO_C_BINDING, ONLY: c_char, c_int, c_null_char
   USE, INTRINSIC :: ISO_FORTRAN_ENV, ONLY: int8, int64
   USE MOD_SPMD_Task, ONLY: CoLM_stop
   IMPLICIT NONE
   PRIVATE
   PUBLIC :: make_directory, copy_file, list_matching_paths, move_file

   INTERFACE
      FUNCTION mkdir_one(path) BIND(C, name="colm_mkdir_one") RESULT(status)
         IMPORT c_char, c_int
         CHARACTER(kind=c_char), INTENT(in) :: path(*)
         INTEGER(c_int) :: status
      END FUNCTION mkdir_one

      FUNCTION list_paths(prefix, suffix, listfile) BIND(C, name="colm_list_matching_paths") RESULT(status)
         IMPORT c_char, c_int
         CHARACTER(kind=c_char), INTENT(in) :: prefix(*), suffix(*), listfile(*)
         INTEGER(c_int) :: status
      END FUNCTION list_paths

      FUNCTION same_file_native(src, dst) BIND(C, name="colm_same_file") RESULT(status)
         IMPORT c_char, c_int
         CHARACTER(kind=c_char), INTENT(in) :: src(*), dst(*)
         INTEGER(c_int) :: status
      END FUNCTION same_file_native

      FUNCTION rename_native(src, dst) BIND(C, name="colm_rename_file") RESULT(status)
         IMPORT c_char, c_int
         CHARACTER(kind=c_char), INTENT(in) :: src(*), dst(*)
         INTEGER(c_int) :: status
      END FUNCTION rename_native
   END INTERFACE

CONTAINS

   ! mkdir -p semantics without invoking a shell. Quotes, spaces, percent signs,
   ! and shell metacharacters are filesystem bytes, not executable commands.
   RECURSIVE SUBROUTINE make_directory(path)
      CHARACTER(len=*), INTENT(in) :: path
      CHARACTER(len=:), ALLOCATABLE :: dir
      CHARACTER(len=12) :: code
      INTEGER :: parent, n
      INTEGER(c_int) :: status

      CALL check_path(path, 'directory')
      dir = trim(path)
      n = len(dir)
      DO WHILE (n > 1)
         IF (.NOT. is_separator(dir(n:n))) EXIT
         n = n - 1
      ENDDO
      dir = dir(:n)
      status = mkdir_one(dir // c_null_char)
      IF (status == 0) RETURN

      parent = n
      DO WHILE (parent > 0)
         IF (is_separator(dir(parent:parent))) EXIT
         parent = parent - 1
      ENDDO
      IF (parent > 0 .AND. parent < n) THEN
         CALL make_directory(dir(:parent))
         status = mkdir_one(dir // c_null_char)
      ENDIF
      IF (status /= 0) THEN
         WRITE(code, '(I0)') status
         CALL CoLM_stop('Cannot create directory "' // dir // '" (errno ' // trim(code) // ')')
      ENDIF
   END SUBROUTINE make_directory

   SUBROUTINE copy_file(src, dst)
      CHARACTER(len=*), INTENT(in) :: src, dst
      INTEGER, PARAMETER :: chunk = 1048576
      INTEGER(int8), ALLOCATABLE :: buffer(:)
      INTEGER(int64) :: nbytes, pos
      INTEGER :: in_unit, out_unit, ios, n
      INTEGER(c_int) :: same
      LOGICAL :: fexists
      CHARACTER(len=256) :: iomsg, code

      CALL check_path(src, 'source file')
      CALL check_path(dst, 'destination file')
      INQUIRE(file=trim(src), exist=fexists, size=nbytes)
      IF (.NOT. fexists) CALL CoLM_stop('Cannot copy missing file "' // trim(src) // '"')
      same = same_file_native(trim(src) // c_null_char, trim(dst) // c_null_char)
      IF (same < 0) THEN
         WRITE(code, '(I0)') -same
         CALL CoLM_stop('Cannot stat copy path "' // trim(src) // '" or "' // trim(dst) // &
            '" (status ' // trim(code) // ')')
      ENDIF
      IF (same /= 0) CALL CoLM_stop('Refusing to copy file onto itself: "' // trim(src) // '"')

      OPEN(newunit=in_unit, file=trim(src), access='stream', form='unformatted', &
         status='old', action='read', iostat=ios, iomsg=iomsg)
      IF (ios /= 0) CALL CoLM_stop('Cannot open source file "' // trim(src) // '": ' // trim(iomsg))

      OPEN(newunit=out_unit, file=trim(dst), access='stream', form='unformatted', &
         status='replace', action='write', iostat=ios, iomsg=iomsg)
      IF (ios /= 0) THEN
         CLOSE(in_unit)
         CALL CoLM_stop('Cannot open destination file "' // trim(dst) // '": ' // trim(iomsg))
      ENDIF

      ALLOCATE(buffer(chunk))
      pos = 1_int64
      DO WHILE (pos <= nbytes)
         n = int(min(int(chunk, int64), nbytes - pos + 1_int64))
         READ(in_unit, pos=pos, iostat=ios, iomsg=iomsg) buffer(:n)
         IF (ios /= 0) THEN
            CLOSE(in_unit); CLOSE(out_unit)
            CALL CoLM_stop('Cannot read source file "' // trim(src) // '": ' // trim(iomsg))
         ENDIF
         WRITE(out_unit, pos=pos, iostat=ios, iomsg=iomsg) buffer(:n)
         IF (ios /= 0) THEN
            CLOSE(in_unit); CLOSE(out_unit)
            CALL CoLM_stop('Cannot write destination file "' // trim(dst) // '": ' // trim(iomsg))
         ENDIF
         pos = pos + n
      ENDDO
      CLOSE(in_unit)
      CLOSE(out_unit, iostat=ios, iomsg=iomsg)
      IF (ios /= 0) CALL CoLM_stop('Cannot close destination file "' // trim(dst) // '": ' // trim(iomsg))
   END SUBROUTINE copy_file

   SUBROUTINE list_matching_paths(prefix, suffix, listfile)
      CHARACTER(len=*), INTENT(in) :: prefix, suffix, listfile
      CHARACTER(len=12) :: code
      INTEGER(c_int) :: status

      CALL check_path(prefix, 'path prefix')
      CALL check_path(suffix, 'path suffix')
      CALL check_path(listfile, 'list file')
      status = list_paths(trim(prefix) // c_null_char, trim(suffix) // c_null_char, &
         trim(listfile) // c_null_char)
      IF (status /= 0) THEN
         WRITE(code, '(I0)') status
         CALL CoLM_stop('Cannot list paths matching "' // trim(prefix) // '*' // trim(suffix) // &
            '" (errno ' // trim(code) // ')')
      ENDIF
   END SUBROUTINE list_matching_paths

   SUBROUTINE move_file(src, dst)
      CHARACTER(len=*), INTENT(in) :: src, dst
      CHARACTER(len=12) :: code
      INTEGER(c_int) :: status

      CALL check_path(src, 'source file')
      CALL check_path(dst, 'destination file')
      status = rename_native(trim(src) // c_null_char, trim(dst) // c_null_char)
      IF (status /= 0) THEN
         WRITE(code, '(I0)') status
         CALL CoLM_stop('Cannot rename "' // trim(src) // '" to "' // trim(dst) // &
            '" (errno ' // trim(code) // ')')
      ENDIF
   END SUBROUTINE move_file

   SUBROUTINE check_path(path, what)
      CHARACTER(len=*), INTENT(in) :: path, what
      IF (len_trim(path) == 0 .OR. index(path, c_null_char) /= 0) THEN
         CALL CoLM_stop('Invalid empty or NUL-containing ' // trim(what) // ' path')
      ENDIF
      IF (index(path, new_line('a')) /= 0) THEN
         CALL CoLM_stop('Invalid newline-containing ' // trim(what) // ' path')
      ENDIF
   END SUBROUTINE check_path

   LOGICAL FUNCTION is_separator(character)
      CHARACTER(len=1), INTENT(in) :: character
      is_separator = character == '/'
#ifdef _WIN32
      is_separator = is_separator .OR. character == achar(92)
#endif
   END FUNCTION is_separator
END MODULE MOD_Filesystem
