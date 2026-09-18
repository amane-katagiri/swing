FROM swing-demo-mirror
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends faketime \
    && rm -rf /var/lib/apt/lists/*
USER swing
