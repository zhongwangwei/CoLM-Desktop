! gen.f90 N SEED：用 gfortran real(16) 生成 binary128 对拍向量（colm-core/src/binary128_tests.rs）。
! 每行：a_hi a_lo b_hi b_lo add_hi add_lo sub_hi sub_lo mul_hi mul_lo div_hi div_lo a_f64 cmp
! 输入 A = a1 + a2、B = b1 + b2 在 real(16) 下相加，覆盖不同量级、相消与平局。
program gen
  implicit none
  integer, parameter :: qp = selected_real_kind(24)
  integer :: n, i, seed_n, cmp
  integer, allocatable :: seed(:)
  character(len=32) :: arg
  real(8) :: u(8), a1, a2, b1, b2
  real(qp) :: a, b
  call get_command_argument(1, arg); read(arg, *) n
  call get_command_argument(2, arg); read(arg, *) seed_n
  call random_seed(size=i); allocate(seed(i)); seed = seed_n; call random_seed(put=seed)
  do i = 1, n
    call random_number(u)
    a1 = pick(u(1), u(2), i)
    a2 = a1 * pick2(u(3), i)
    b1 = pick(u(4), u(5), i + 7)
    b2 = b1 * pick2(u(6), i + 3)
    if (mod(i, 11) == 0) b1 = a1 * (1.0d0 + u(7) * 1.0d-14)   ! 近似相消
    if (mod(i, 13) == 0) b1 = -a1
    a = real(a1, qp) + real(a2, qp)
    b = real(b1, qp) + real(b2, qp)
    if (a < b) then
      cmp = -1
    else if (a > b) then
      cmp = 1
    else
      cmp = 0
    end if
    write (*, '(13(Z16.16,1X),I2)') hx(a), hx(b), hx(a + b), hx(a - b), hx(a * b), hx(a / b), &
      transfer(real(a, 8), 0_8), cmp
  end do
contains
  function pick(x, y, k) result(v)
    real(8), intent(in) :: x, y
    integer, intent(in) :: k
    real(8) :: v
    v = (x - 0.5d0) * 10.0d0 ** int((y - 0.5d0) * 40.0d0)
    if (mod(k, 17) == 0) v = (x - 0.5d0) * 10.0d0 ** int((y - 0.5d0) * 600.0d0)
  end function
  function pick2(x, k) result(v)
    real(8), intent(in) :: x
    integer, intent(in) :: k
    real(8) :: v
    v = (x - 0.5d0) * 2.0d0 ** (-int(x * 120.0d0))
    if (mod(k, 5) == 0) v = 0.0d0
    if (mod(k, 7) == 0) v = 2.0d0 ** (-53) * sign(1.0d0, x - 0.5d0)    ! 平局附近
  end function
  function hx(q) result(w)
    real(qp), intent(in) :: q
    integer(8) :: w(2), t(2)
    t = transfer(q, t)
    w(1) = t(2); w(2) = t(1)   ! 高 64 位在前
  end function
end program
