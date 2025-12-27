FROM oraclelinux:7-slim

# 安装基础工具和依赖
RUN yum install -y \
    gcc \
    gcc-c++ \
    make \
    openssl-devel \
    curl \
    wget \
    unzip \
    libaio gcc gcc-c++ make openssl-devel tar gzip findutils \
    && yum clean all

# 安装Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# 安装Oracle Instant Client
RUN yum -y install oracle-instantclient-release-el7 && \
    yum -y install oracle-instantclient-basic oracle-instantclient-devel && \
    yum clean all

# 设置Oracle环境变量
RUN echo "export LD_LIBRARY_PATH=\$(find /usr/lib/oracle -name client64)/lib" >> /etc/profile && \
    echo "export LD_LIBRARY_PATH=\$(find /usr/lib/oracle -name client64)/lib" >> $HOME/.bashrc


# 创建工作目录
WORKDIR /build

# 复制项目文件
COPY . .

# 编译项目并输出结果
CMD ["/bin/bash", "-c", "source $HOME/.bashrc && cargo build --release && cp target/release/el /output/"]
