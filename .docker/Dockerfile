FROM docker.io/debian:10

LABEL authors="Onur Özkan <onur@komodoplatform.com>"

RUN apt-get update -y

RUN apt-get install -y 	\
	build-essential 	\
	cmake 			 	\
    ca-certificates 	\
    curl             	\
    wget             	\
    unzip             	\
	libudev-dev			\
    gnupg

RUN ln -s /usr/bin/python3 /bin/python

RUN apt install -y  			\
	software-properties-common 	\
	lsb-release

RUN curl --output llvm.sh https://apt.llvm.org/llvm.sh

RUN chmod +x llvm.sh

RUN ./llvm.sh 16

RUN rm ./llvm.sh

ENV AR=/usr/bin/llvm-ar-16
ENV CC=/usr/bin/clang-16

RUN ln -s /usr/bin/clang-16 /usr/bin/clang

RUN mkdir -m 0755 -p /etc/apt/keyrings

RUN curl -fsSL https://download.docker.com/linux/debian/gpg | gpg --dearmor -o /etc/apt/keyrings/docker.gpg

RUN echo \
  "deb [arch="$(dpkg --print-architecture)" signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/debian \
  "$(. /etc/os-release && echo "$VERSION_CODENAME")" stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null

RUN apt-get update -y

RUN apt-get install -y 	  \
	docker-ce 			  \
	docker-ce-cli 		  \
	containerd.io 		  \
	docker-buildx-plugin

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --profile minimal --default-toolchain nightly-2023-06-01 -y
ENV PATH="/root/.cargo/bin:$PATH"
# TODO: Lock wasm-pack version
RUN curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | bash -s -- -y
RUN wget https://github.com/protocolbuffers/protobuf/releases/download/v25.3/protoc-25.3-linux-x86_64.zip
RUN unzip protoc-25.3-linux-x86_64.zip && mv ./include/google /usr/include/google

