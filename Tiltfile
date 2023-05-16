def crd_yaml():
    read_file('src/crdgen.rs')
    read_file('Cargo.toml')
    read_file('Cargo.lock')
    read_file('src/controller/crd.rs')
    return local('cargo run --bin crdgen')

def namespace_yaml(name):
    return blob("""
    kind: Namespace
    apiVersion: v1
    metadata:
      name: %s
    spec:  {}""" % name)

k8s_yaml(crd_yaml())
k8s_yaml(namespace_yaml('a'))
k8s_yaml(namespace_yaml('b'))
k8s_yaml('test-deploy/secret-remote.yml')
k8s_yaml('test-deploy/shared-secret.yml')
k8s_yaml('test-deploy/shared-secret-request.yml')
k8s_resource(objects=['sharedsecrets.sharedsecretctl.klauser.link:customresourcedefinition', 'sharedsecretrequests.sharedsecretctl.klauser.link:customresourcedefinition'], new_name='crd')
k8s_resource(objects=['a:Namespace', 'remote:Secret:a', 'remote:SharedSecret:a'], new_name='a')
k8s_resource(objects=['b:Namespace', 'local:SharedSecretRequest:b'], new_name='b')
