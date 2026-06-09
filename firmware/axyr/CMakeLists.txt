if(CONFIG_AXYR)
  zephyr_library()
  zephyr_library_sources(src/axyr_trace.c)
endif()
