#ifndef FLUIDITY_H
#define FLUIDITY_H

char *fluidity_call(const char *method, const char *params);
void fluidity_free(char *value);

#endif
