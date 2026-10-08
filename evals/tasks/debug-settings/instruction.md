A user reports that with this settings file, retries stay at 3 instead of 5:

```
# app settings
Timeout = 30
max_retries = 5
```

Find the cause and fix it, and add a regression test. Don't change the existing tests.
