#!/usr/bin/env bash
awk 'BEGIN { msg="—"; print msg }'
awk 'BEGIN { re="—"; re="x"; if ($0 ~ re) print }'
awk 'BEGIN { if (c) re="—"; if ($0 ~ re) print }'
awk 'BEGIN { re = "—" tail; if ($0 ~ re) print }'
awk 'BEGIN { c && (re = "—"); if ($0 ~ re) print }'
awk 'BEGIN { switch (x) { case 1: re = "—"; break } if ($0 ~ re) print }'
