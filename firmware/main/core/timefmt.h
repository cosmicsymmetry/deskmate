#pragma once
void timefmt_hhmm(char out[6], int hour, int minute);
// dow: 0 = Monday ... 6 = Sunday
void timefmt_date(char out[32], int year, int month, int day, int dow);
