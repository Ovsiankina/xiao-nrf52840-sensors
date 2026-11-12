MEMORY
{
  /* Seeed XIAO nRF52840 Sense. The factory bootloader + SoftDevice S140
     7.3.0 sit below 0x27000 and stay there; we never enable the
     SoftDevice, so it keeps only the first 8 bytes of RAM. The
     bootloader refuses writes at or above 0xEA000.
     Source: docs/research/sensor-crates.md. */
  FLASH : ORIGIN = 0x00027000, LENGTH = 0xC3000
  RAM   : ORIGIN = 0x20000008, LENGTH = 256K - 8
}
